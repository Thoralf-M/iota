// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

pub use checked::*;
#[iota_macros::with_checked_arithmetic]
mod checked {
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

    use anyhow::Result;
    use iota_move_natives::{NativesCostTable, object_runtime, object_runtime::ObjectRuntime};
    use iota_protocol_config::ProtocolConfig;
    use iota_types::{
        base_types::*,
        error::{ExecutionError, ExecutionErrorKind, IotaError},
        execution_config_utils::to_binary_config,
        metrics::{BytecodeVerifierMetrics, LimitsMetrics},
        storage::ChildObjectResolver,
    };
    use iota_verifier::{
        check_for_verifier_timeout, verifier::iota_verify_module_metered_check_timeout_only,
    };
    use move_binary_format::file_format::CompiledModule;
    use move_bytecode_verifier::verify_module_with_config_metered;
    use move_bytecode_verifier_meter::{Meter, Scope};
    use move_core_types::account_address::AccountAddress;
    #[cfg(feature = "tracing")]
    use move_vm_config::runtime::VMProfilerConfig;
    use move_vm_config::{
        runtime::{VMConfig, VMRuntimeLimitsConfig},
        verifier::VerifierConfig,
    };
    use move_vm_runtime::{
        move_vm::MoveVM, native_extensions::NativeContextExtensions,
        native_functions::NativeFunctionTable,
    };
    use tracing::instrument;

    /// Creates a new instance of `MoveVM` with the specified native functions
    /// and protocol configuration. The VM is configured using a `VMConfig`
    /// that sets limits for vector length, value depth, and other
    /// runtime options based on the provided `ProtocolConfig`. If gas profiling
    /// is enabled, the function configures the profiler with the provided
    /// path.
    pub fn new_move_vm(
        natives: NativeFunctionTable,
        protocol_config: &ProtocolConfig,
        _enable_profiler: Option<PathBuf>,
    ) -> Result<MoveVM, IotaError> {
        #[cfg(not(feature = "tracing"))]
        let vm_profiler_config = None;
        #[cfg(feature = "tracing")]
        let vm_profiler_config = _enable_profiler.clone().map(|path| VMProfilerConfig {
            full_path: path,
            track_bytecode_instructions: false,
            use_long_function_name: false,
        });
        MoveVM::new_with_config(
            natives,
            VMConfig {
                verifier: protocol_config.verifier_config(/* signing_limits */ None),
                max_binary_format_version: protocol_config.move_binary_format_version(),
                runtime_limits_config: VMRuntimeLimitsConfig {
                    vector_len_max: protocol_config.max_move_vector_len(),
                    max_value_nest_depth: protocol_config.max_move_value_depth_as_option(),
                    hardened_otw_check: protocol_config.hardened_otw_check(),
                },
                enable_invariant_violation_check_in_swap_loc: !protocol_config
                    .disable_invariant_violation_check_in_swap_loc(),
                check_no_extraneous_bytes_during_deserialization: protocol_config
                    .no_extraneous_module_bytes(),
                profiler_config: vm_profiler_config,
                // Don't augment errors with execution state on-chain
                error_execution_state: false,
                binary_config: to_binary_config(protocol_config),
                rethrow_serialization_type_layout_errors: protocol_config
                    .rethrow_serialization_type_layout_errors(),
                max_type_to_layout_nodes: protocol_config.max_type_to_layout_nodes_as_option(),
                variant_nodes: protocol_config.variant_nodes(),
            },
        )
        .map_err(|_| IotaError::ExecutionInvariantViolation)
    }

    /// Creates a new set of `NativeContextExtensions` for the Move VM,
    /// configuring extensions such as `ObjectRuntime` and
    /// `NativesCostTable`. These extensions manage object resolution, input
    /// objects, metering, protocol configuration, and metrics tracking.
    /// They are available and mainly used in native function implementations
    /// via `NativeContext` instance.
    pub fn new_native_extensions<'r>(
        child_resolver: &'r dyn ChildObjectResolver,
        input_objects: BTreeMap<ObjectID, object_runtime::InputObject>,
        is_metered: bool,
        protocol_config: &'r ProtocolConfig,
        metrics: Arc<LimitsMetrics>,
        current_epoch_id: EpochId,
    ) -> NativeContextExtensions<'r> {
        let mut extensions = NativeContextExtensions::default();
        extensions.add(ObjectRuntime::new(
            child_resolver,
            input_objects,
            is_metered,
            protocol_config,
            metrics,
            current_epoch_id,
        ));
        extensions.add(NativesCostTable::from_protocol_config(protocol_config));
        extensions
    }

    /// Given a list of `modules` and an `object_id`, mutate each module's self
    /// ID (which must be 0x0) to be `object_id`.
    pub fn substitute_package_id(
        modules: &mut [CompiledModule],
        object_id: ObjectID,
    ) -> Result<(), ExecutionError> {
        let new_address = AccountAddress::from(object_id);

        for module in modules.iter_mut() {
            let self_handle = module.self_handle().clone();
            let self_address_idx = self_handle.address;

            let addrs = &mut module.address_identifiers;
            let Some(address_mut) = addrs.get_mut(self_address_idx.0 as usize) else {
                let name = module.identifier_at(self_handle.name);
                return Err(ExecutionError::new_with_source(
                    ExecutionErrorKind::PublishErrorNonZeroAddress,
                    format!("Publishing module {name} with invalid address index"),
                ));
            };

            if *address_mut != AccountAddress::ZERO {
                let name = module.identifier_at(self_handle.name);
                return Err(ExecutionError::new_with_source(
                    ExecutionErrorKind::PublishErrorNonZeroAddress,
                    format!("Publishing module {name} with non-zero address is not allowed"),
                ));
            };

            *address_mut = new_address;
        }

        Ok(())
    }

    /// Run the bytecode verifier with a meter limit
    ///
    /// This function only fails if the verification does not complete within
    /// the limit.  If the modules fail to verify but verification completes
    /// within the meter limit, the function succeeds.
    #[instrument(level = "trace", skip_all)]
    pub fn run_metered_move_bytecode_verifier(
        modules: &[CompiledModule],
        verifier_config: &VerifierConfig,
        meter: &mut (impl Meter + ?Sized),
        metrics: &Arc<BytecodeVerifierMetrics>,
    ) -> Result<(), IotaError> {
        // run the Move verifier
        for module in modules.iter() {
            let per_module_meter_verifier_timer = metrics
                .verifier_runtime_per_module_success_latency
                .start_timer();

            if let Err(e) = verify_module_timeout_only(module, verifier_config, meter) {
                // We only checked that the failure was due to timeout
                // Discard success timer, but record timeout/failure timer
                metrics
                    .verifier_runtime_per_module_timeout_latency
                    .observe(per_module_meter_verifier_timer.stop_and_discard());
                metrics
                    .verifier_timeout_metrics
                    .with_label_values(&[
                        BytecodeVerifierMetrics::OVERALL_TAG,
                        BytecodeVerifierMetrics::TIMEOUT_TAG,
                    ])
                    .inc();

                return Err(e);
            };

            // Save the success timer
            per_module_meter_verifier_timer.stop_and_record();
            metrics
                .verifier_timeout_metrics
                .with_label_values(&[
                    BytecodeVerifierMetrics::OVERALL_TAG,
                    BytecodeVerifierMetrics::SUCCESS_TAG,
                ])
                .inc();
        }

        Ok(())
    }

    /// Run both the Move verifier and the IOTA verifier, checking just for
    /// timeouts. Returns Ok(()) if the verifier completes within the module
    /// meter limit and the ticks are successfully transferred to the package
    /// limit (regardless of whether verification succeeds or not).
    fn verify_module_timeout_only(
        module: &CompiledModule,
        verifier_config: &VerifierConfig,
        meter: &mut (impl Meter + ?Sized),
    ) -> Result<(), IotaError> {
        meter.enter_scope(module.self_id().name().as_str(), Scope::Module);

        if let Err(e) = verify_module_with_config_metered(verifier_config, module, meter) {
            // Check that the status indicates metering timeout.
            if check_for_verifier_timeout(&e.major_status()) {
                return Err(IotaError::ModuleVerificationFailure {
                    error: format!("Verification timed out: {}", e),
                });
            }
        } else if let Err(err) =
            iota_verify_module_metered_check_timeout_only(module, &BTreeMap::new(), meter)
        {
            return Err(err.into());
        }

        if meter.transfer(Scope::Module, Scope::Package, 1.0).is_err() {
            return Err(IotaError::ModuleVerificationFailure {
                error: "Verification timed out".to_string(),
            });
        }

        Ok(())
    }
}
