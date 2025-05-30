// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use move_core_types::account_address::AccountAddress;
use move_vm_config::{runtime::VMConfig, verifier::VerifierConfig};
use move_vm_runtime::move_vm::MoveVM;
use move_vm_test_utils::InMemoryStorage;
use move_vm_types::gas::UnmeteredGasMeter;

use crate::compiler::{as_module, compile_units, serialize_module_at_max_version};

const TEST_ADDR: AccountAddress = AccountAddress::new([42; AccountAddress::LENGTH]);

#[test]
fn test_publish_module_with_nested_loops() {
    // Compile the modules and scripts.
    // TODO: find a better way to include the Signer module.
    let code = r#"
        module {{ADDR}}::M {
            fun foo() {
                let mut i = 0;
                while (i < 10) {
                    let mut j = 0;
                    while (j < 10) {
                        j = j + 1;
                    };
                    i = i + 1;
                };
            }
        }
    "#;
    let code = code.replace("{{ADDR}}", &format!("0x{}", TEST_ADDR));
    let mut units = compile_units(&code).unwrap();

    let m = as_module(units.pop().unwrap());
    let mut m_blob = vec![];
    serialize_module_at_max_version(&m, &mut m_blob).unwrap();

    // Should succeed with max_loop_depth = 2
    {
        let storage = InMemoryStorage::new();
        let vm = MoveVM::new_with_config(
            move_stdlib_natives::all_natives(
                AccountAddress::from_hex_literal("0x1").unwrap(),
                move_stdlib_natives::GasParameters::zeros(),
                // silent debug
                true,
            ),
            VMConfig {
                verifier: VerifierConfig {
                    max_loop_depth: Some(2),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();

        let mut sess = vm.new_session(&storage);
        sess.publish_module(m_blob.clone(), TEST_ADDR, &mut UnmeteredGasMeter)
            .unwrap();
    }

    // Should fail with max_loop_depth = 1
    {
        let storage = InMemoryStorage::new();
        let vm = MoveVM::new_with_config(
            move_stdlib_natives::all_natives(
                AccountAddress::from_hex_literal("0x1").unwrap(),
                move_stdlib_natives::GasParameters::zeros(),
                // silent debug
                true,
            ),
            VMConfig {
                verifier: VerifierConfig {
                    max_loop_depth: Some(1),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();

        let mut sess = vm.new_session(&storage);
        sess.publish_module(m_blob, TEST_ADDR, &mut UnmeteredGasMeter)
            .unwrap_err();
    }
}
