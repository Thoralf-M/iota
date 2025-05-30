// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{iter, mem, ops::Not, sync::Arc, thread};

use either::Either;
use fastcrypto::hash::{HashFunction, Sha3_256};
use futures::stream::FuturesUnordered;
use iota_common::sync::notify_read::NotifyRead;
use iota_config::{migration_tx_data::MigrationTxData, node::AuthorityStorePruningConfig};
use iota_macros::fail_point_arg;
use iota_storage::mutex_table::{MutexGuard, MutexTable, RwLockGuard, RwLockTable};
use iota_types::{
    accumulator::Accumulator,
    base_types::SequenceNumber,
    digests::TransactionEventsDigest,
    effects::{TransactionEffects, TransactionEvents},
    error::UserInputError,
    execution::TypeLayoutStore,
    fp_bail, fp_ensure,
    iota_system_state::{
        get_iota_system_state, iota_system_state_summary::IotaSystemStateSummaryV2,
    },
    message_envelope::Message,
    storage::{
        BackingPackageStore, MarkerValue, ObjectKey, ObjectOrTombstone, ObjectStore, get_module,
    },
};
use itertools::izip;
use move_core_types::resolver::ModuleResolver;
use tokio::{
    sync::{RwLockReadGuard, RwLockWriteGuard},
    time::Instant,
};
use tracing::{debug, info, trace};
use typed_store::{
    TypedStoreError,
    rocks::{DBBatch, DBMap, util::is_ref_count_value},
    traits::Map,
};

use super::{
    authority_store_tables::{AuthorityPerpetualTables, LiveObject},
    *,
};
use crate::{
    authority::{
        authority_per_epoch_store::{AuthorityPerEpochStore, LockDetails},
        authority_store_pruner::{
            AuthorityStorePruner, AuthorityStorePruningMetrics, EPOCH_DURATION_MS_FOR_TESTING,
        },
        authority_store_tables::TotalIotaSupplyCheck,
        authority_store_types::{
            ObjectContentDigest, StoreObject, StoreObjectPair, StoreObjectWrapper,
            get_store_object_pair,
        },
        epoch_start_configuration::{EpochFlag, EpochStartConfiguration},
    },
    rest_index::RestIndexStore,
    state_accumulator::AccumulatorStore,
    transaction_outputs::TransactionOutputs,
};

const NUM_SHARDS: usize = 4096;

struct AuthorityStoreMetrics {
    iota_conservation_check_latency: IntGauge,
    iota_conservation_live_object_count: IntGauge,
    iota_conservation_live_object_size: IntGauge,
    iota_conservation_imbalance: IntGauge,
    iota_conservation_storage_fund: IntGauge,
    iota_conservation_storage_fund_imbalance: IntGauge,
    epoch_flags: IntGaugeVec,
}

impl AuthorityStoreMetrics {
    pub fn new(registry: &Registry) -> Self {
        Self {
            iota_conservation_check_latency: register_int_gauge_with_registry!(
                "iota_conservation_check_latency",
                "Number of seconds took to scan all live objects in the store for IOTA conservation check",
                registry,
            ).unwrap(),
            iota_conservation_live_object_count: register_int_gauge_with_registry!(
                "iota_conservation_live_object_count",
                "Number of live objects in the store",
                registry,
            ).unwrap(),
            iota_conservation_live_object_size: register_int_gauge_with_registry!(
                "iota_conservation_live_object_size",
                "Size in bytes of live objects in the store",
                registry,
            ).unwrap(),
            iota_conservation_imbalance: register_int_gauge_with_registry!(
                "iota_conservation_imbalance",
                "Total amount of IOTA in the network - 10B * 10^9. This delta shows the amount of imbalance",
                registry,
            ).unwrap(),
            iota_conservation_storage_fund: register_int_gauge_with_registry!(
                "iota_conservation_storage_fund",
                "Storage Fund pool balance (only includes the storage fund proper that represents object storage)",
                registry,
            ).unwrap(),
            iota_conservation_storage_fund_imbalance: register_int_gauge_with_registry!(
                "iota_conservation_storage_fund_imbalance",
                "Imbalance of storage fund, computed with storage_fund_balance - total_object_storage_rebates",
                registry,
            ).unwrap(),
            epoch_flags: register_int_gauge_vec_with_registry!(
                "epoch_flags",
                "Local flags of the currently running epoch",
                &["flag"],
                registry,
            ).unwrap(),
        }
    }
}

/// The `AuthorityStore` manages the state and operations of an authority's
/// store. It includes a `mutex_table` to handle concurrent writes to the
/// database and references to various tables stored in
/// `AuthorityPerpetualTables`. The struct provides mechanisms for initializing
/// and accessing locks, managing objects and transactions, and performing
/// epoch-specific operations. It also includes methods for recovering from
/// crashes, checking IOTA conservation, and handling object markers and states
/// during epoch transitions.
pub struct AuthorityStore {
    /// Internal vector of locks to manage concurrent writes to the database
    mutex_table: MutexTable<ObjectDigest>,

    pub(crate) perpetual_tables: Arc<AuthorityPerpetualTables>,

    pub(crate) root_state_notify_read: NotifyRead<EpochId, (CheckpointSequenceNumber, Accumulator)>,

    /// Guards reference count updates to `indirect_move_objects` table
    pub(crate) objects_lock_table: Arc<RwLockTable<ObjectContentDigest>>,

    indirect_objects_threshold: usize,

    /// Whether to enable expensive IOTA conservation check at epoch boundaries.
    enable_epoch_iota_conservation_check: bool,

    metrics: AuthorityStoreMetrics,
}

pub type ExecutionLockReadGuard<'a> = RwLockReadGuard<'a, EpochId>;
pub type ExecutionLockWriteGuard<'a> = RwLockWriteGuard<'a, EpochId>;

impl AuthorityStore {
    /// Open an authority store by directory path.
    /// If the store is empty, initialize it using genesis.
    pub async fn open(
        perpetual_tables: Arc<AuthorityPerpetualTables>,
        genesis: &Genesis,
        config: &NodeConfig,
        registry: &Registry,
        migration_tx_data: Option<&MigrationTxData>,
    ) -> IotaResult<Arc<Self>> {
        let indirect_objects_threshold = config.indirect_objects_threshold;
        let enable_epoch_iota_conservation_check = config
            .expensive_safety_check_config
            .enable_epoch_iota_conservation_check();

        let epoch_start_configuration = if perpetual_tables.database_is_empty()? {
            info!("Creating new epoch start config from genesis");

            #[cfg_attr(not(any(msim, fail_points)), expect(unused_mut))]
            let mut initial_epoch_flags = EpochFlag::default_flags_for_new_epoch(config);
            fail_point_arg!("initial_epoch_flags", |flags: Vec<EpochFlag>| {
                info!("Setting initial epoch flags to {:?}", flags);
                initial_epoch_flags = flags;
            });

            let epoch_start_configuration = EpochStartConfiguration::new(
                genesis.iota_system_object().into_epoch_start_state(),
                *genesis.checkpoint().digest(),
                &genesis.objects(),
                initial_epoch_flags,
            )?;
            perpetual_tables.set_epoch_start_configuration(&epoch_start_configuration)?;
            epoch_start_configuration
        } else {
            info!("Loading epoch start config from DB");
            perpetual_tables
                .epoch_start_configuration
                .get(&())?
                .expect("Epoch start configuration must be set in non-empty DB")
        };
        let cur_epoch = perpetual_tables.get_recovery_epoch_at_restart()?;
        info!("Epoch start config: {:?}", epoch_start_configuration);
        info!("Cur epoch: {:?}", cur_epoch);
        let this = Self::open_inner(
            genesis,
            perpetual_tables,
            indirect_objects_threshold,
            enable_epoch_iota_conservation_check,
            registry,
            migration_tx_data,
        )
        .await?;
        this.update_epoch_flags_metrics(&[], epoch_start_configuration.flags());
        Ok(this)
    }

    pub fn update_epoch_flags_metrics(&self, old: &[EpochFlag], new: &[EpochFlag]) {
        for flag in old {
            self.metrics
                .epoch_flags
                .with_label_values(&[&flag.to_string()])
                .set(0);
        }
        for flag in new {
            self.metrics
                .epoch_flags
                .with_label_values(&[&flag.to_string()])
                .set(1);
        }
    }

    // NB: This must only be called at time of reconfiguration. We take the
    // execution lock write guard as an argument to ensure that this is the
    // case.
    pub fn clear_object_per_epoch_marker_table(
        &self,
        _execution_guard: &ExecutionLockWriteGuard<'_>,
    ) -> IotaResult<()> {
        // We can safely delete all entries in the per epoch marker table since this is
        // only called at epoch boundaries (during reconfiguration). Therefore
        // any entries that currently exist can be removed. Because of this we
        // can use the `schedule_delete_all` method.
        Ok(self
            .perpetual_tables
            .object_per_epoch_marker_table
            .schedule_delete_all()?)
    }

    pub async fn open_with_committee_for_testing(
        perpetual_tables: Arc<AuthorityPerpetualTables>,
        committee: &Committee,
        genesis: &Genesis,
        indirect_objects_threshold: usize,
    ) -> IotaResult<Arc<Self>> {
        // TODO: Since we always start at genesis, the committee should be technically
        // the same as the genesis committee.
        assert_eq!(committee.epoch, 0);
        Self::open_inner(
            genesis,
            perpetual_tables,
            indirect_objects_threshold,
            true,
            &Registry::new(),
            None,
        )
        .await
    }

    async fn open_inner(
        genesis: &Genesis,
        perpetual_tables: Arc<AuthorityPerpetualTables>,
        indirect_objects_threshold: usize,
        enable_epoch_iota_conservation_check: bool,
        registry: &Registry,
        migration_tx_data: Option<&MigrationTxData>,
    ) -> IotaResult<Arc<Self>> {
        let store = Arc::new(Self {
            mutex_table: MutexTable::new(NUM_SHARDS),
            perpetual_tables,
            root_state_notify_read:
                NotifyRead::<EpochId, (CheckpointSequenceNumber, Accumulator)>::new(),
            objects_lock_table: Arc::new(RwLockTable::new(NUM_SHARDS)),
            indirect_objects_threshold,
            enable_epoch_iota_conservation_check,
            metrics: AuthorityStoreMetrics::new(registry),
        });
        // Only initialize an empty database.
        if store
            .database_is_empty()
            .expect("database read should not fail at init.")
        {
            // Initialize with genesis data
            // First insert genesis objects
            store
                .bulk_insert_genesis_objects(genesis.objects())
                .expect("cannot bulk insert genesis objects");

            // Then insert txn and effects of genesis
            let transaction = VerifiedTransaction::new_unchecked(genesis.transaction().clone());
            store
                .perpetual_tables
                .transactions
                .insert(transaction.digest(), transaction.serializable_ref())
                .expect("cannot insert genesis transaction");
            store
                .perpetual_tables
                .effects
                .insert(&genesis.effects().digest(), genesis.effects())
                .expect("cannot insert genesis effects");

            // In the previous step we don't insert the effects to executed_effects yet
            // because the genesis tx hasn't but will be executed. This is
            // important for fullnodes to be able to generate indexing data
            // right now.
            let event_digests = genesis.events().digest();
            let events = genesis
                .events()
                .data
                .iter()
                .enumerate()
                .map(|(i, e)| ((event_digests, i), e));
            store.perpetual_tables.events.multi_insert(events).unwrap();

            // Initialize with migration data if genesis contained migration transactions
            if let Some(migration_transactions) = migration_tx_data {
                // This migration data was validated during the loading into the node (invoked
                // by the caller of this function)
                let txs_data = migration_transactions.txs_data();

                // We iterate over the contents of the genesis checkpoint, that includes all
                // migration transactions execution digest. Thus we cover all transactions that
                // were considered during the creation of the genesis blob.
                for (_, execution_digest) in genesis
                    .checkpoint_contents()
                    .enumerate_transactions(&genesis.checkpoint())
                {
                    let tx_digest = &execution_digest.transaction;
                    // We can skip the genesis transaction and its data because above it was already
                    // stored in the perpetual_tables.
                    if tx_digest == genesis.transaction().digest() {
                        continue;
                    }
                    // Now we can store in the perpetual_tables this migration transaction, together
                    // with its effects, events and created objects.
                    let Some((tx, effects, events)) = txs_data.get(tx_digest) else {
                        panic!("tx digest not found in migrated objects blob");
                    };
                    let transaction = VerifiedTransaction::new_unchecked(tx.clone());
                    let objects = migration_transactions
                        .objects_by_tx_digest(*tx_digest)
                        .expect("the migration data is corrupted");
                    store
                        .bulk_insert_genesis_objects(&objects)
                        .expect("cannot bulk insert migrated objects");
                    store
                        .perpetual_tables
                        .transactions
                        .insert(transaction.digest(), transaction.serializable_ref())
                        .expect("cannot insert migration transaction");
                    store
                        .perpetual_tables
                        .effects
                        .insert(&effects.digest(), effects)
                        .expect("cannot insert migration effects");
                    let events = events
                        .data
                        .iter()
                        .enumerate()
                        .map(|(i, e)| ((events.digest(), i), e));
                    store.perpetual_tables.events.multi_insert(events).unwrap();
                }
            }
        }

        Ok(store)
    }

    /// Open authority store without any operations that require
    /// genesis, such as constructing EpochStartConfiguration
    /// or inserting genesis objects.
    pub fn open_no_genesis(
        perpetual_tables: Arc<AuthorityPerpetualTables>,
        indirect_objects_threshold: usize,
        enable_epoch_iota_conservation_check: bool,
        registry: &Registry,
    ) -> IotaResult<Arc<Self>> {
        let store = Arc::new(Self {
            mutex_table: MutexTable::new(NUM_SHARDS),
            perpetual_tables,
            root_state_notify_read:
                NotifyRead::<EpochId, (CheckpointSequenceNumber, Accumulator)>::new(),
            objects_lock_table: Arc::new(RwLockTable::new(NUM_SHARDS)),
            indirect_objects_threshold,
            enable_epoch_iota_conservation_check,
            metrics: AuthorityStoreMetrics::new(registry),
        });
        Ok(store)
    }

    pub fn get_recovery_epoch_at_restart(&self) -> IotaResult<EpochId> {
        self.perpetual_tables.get_recovery_epoch_at_restart()
    }

    pub fn get_effects(
        &self,
        effects_digest: &TransactionEffectsDigest,
    ) -> IotaResult<Option<TransactionEffects>> {
        Ok(self.perpetual_tables.effects.get(effects_digest)?)
    }

    /// Returns true if we have an effects structure for this transaction digest
    pub fn effects_exists(&self, effects_digest: &TransactionEffectsDigest) -> IotaResult<bool> {
        self.perpetual_tables
            .effects
            .contains_key(effects_digest)
            .map_err(|e| e.into())
    }

    pub fn get_events(
        &self,
        event_digest: &TransactionEventsDigest,
    ) -> Result<Option<TransactionEvents>, TypedStoreError> {
        let data = self
            .perpetual_tables
            .events
            .safe_range_iter((*event_digest, 0)..=(*event_digest, usize::MAX))
            .map_ok(|(_, event)| event)
            .collect::<Result<Vec<_>, TypedStoreError>>()?;
        Ok(data.is_empty().not().then_some(TransactionEvents { data }))
    }

    pub fn multi_get_events(
        &self,
        event_digests: &[TransactionEventsDigest],
    ) -> IotaResult<Vec<Option<TransactionEvents>>> {
        Ok(event_digests
            .iter()
            .map(|digest| self.get_events(digest))
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn multi_get_effects<'a>(
        &self,
        effects_digests: impl Iterator<Item = &'a TransactionEffectsDigest>,
    ) -> IotaResult<Vec<Option<TransactionEffects>>> {
        Ok(self.perpetual_tables.effects.multi_get(effects_digests)?)
    }

    pub fn get_executed_effects(
        &self,
        tx_digest: &TransactionDigest,
    ) -> IotaResult<Option<TransactionEffects>> {
        let effects_digest = self.perpetual_tables.executed_effects.get(tx_digest)?;
        match effects_digest {
            Some(digest) => Ok(self.perpetual_tables.effects.get(&digest)?),
            None => Ok(None),
        }
    }

    /// Given a list of transaction digests, returns a list of the corresponding
    /// effects only if they have been executed. For transactions that have
    /// not been executed, None is returned.
    pub fn multi_get_executed_effects_digests(
        &self,
        digests: &[TransactionDigest],
    ) -> IotaResult<Vec<Option<TransactionEffectsDigest>>> {
        Ok(self.perpetual_tables.executed_effects.multi_get(digests)?)
    }

    /// Given a list of transaction digests, returns a list of the corresponding
    /// effects only if they have been executed. For transactions that have
    /// not been executed, None is returned.
    pub fn multi_get_executed_effects(
        &self,
        digests: &[TransactionDigest],
    ) -> IotaResult<Vec<Option<TransactionEffects>>> {
        let executed_effects_digests = self.perpetual_tables.executed_effects.multi_get(digests)?;
        let effects = self.multi_get_effects(executed_effects_digests.iter().flatten())?;
        let mut tx_to_effects_map = effects
            .into_iter()
            .flatten()
            .map(|effects| (*effects.transaction_digest(), effects))
            .collect::<HashMap<_, _>>();
        Ok(digests
            .iter()
            .map(|digest| tx_to_effects_map.remove(digest))
            .collect())
    }

    pub fn is_tx_already_executed(&self, digest: &TransactionDigest) -> IotaResult<bool> {
        Ok(self
            .perpetual_tables
            .executed_effects
            .contains_key(digest)?)
    }

    pub fn get_marker_value(
        &self,
        object_id: &ObjectID,
        version: &SequenceNumber,
        epoch_id: EpochId,
    ) -> IotaResult<Option<MarkerValue>> {
        let object_key = (epoch_id, ObjectKey(*object_id, *version));
        Ok(self
            .perpetual_tables
            .object_per_epoch_marker_table
            .get(&object_key)?)
    }

    pub fn get_latest_marker(
        &self,
        object_id: &ObjectID,
        epoch_id: EpochId,
    ) -> IotaResult<Option<(SequenceNumber, MarkerValue)>> {
        let min_key = (epoch_id, ObjectKey::min_for_id(object_id));
        let max_key = (epoch_id, ObjectKey::max_for_id(object_id));

        let marker_entry = self
            .perpetual_tables
            .object_per_epoch_marker_table
            .safe_iter_with_bounds(Some(min_key), Some(max_key))
            .skip_prior_to(&max_key)?
            .next();
        match marker_entry {
            Some(Ok(((epoch, key), marker))) => {
                // because of the iterator bounds these cannot fail
                assert_eq!(epoch, epoch_id);
                assert_eq!(key.0, *object_id);
                Ok(Some((key.1, marker)))
            }
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Returns future containing the state hash for the given epoch
    /// once available
    pub async fn notify_read_root_state_hash(
        &self,
        epoch: EpochId,
    ) -> IotaResult<(CheckpointSequenceNumber, Accumulator)> {
        // We need to register waiters _before_ reading from the database to avoid race
        // conditions
        let registration = self.root_state_notify_read.register_one(&epoch);
        let hash = self.perpetual_tables.root_state_hash_by_epoch.get(&epoch)?;

        let result = match hash {
            // Note that Some() clause also drops registration that is already fulfilled
            Some(ready) => Either::Left(futures::future::ready(ready)),
            None => Either::Right(registration),
        }
        .await;

        Ok(result)
    }

    // Implementation of the corresponding method of `CheckpointCache` trait.
    pub(crate) fn insert_finalized_transactions_perpetual_checkpoints(
        &self,
        digests: &[TransactionDigest],
        epoch: EpochId,
        sequence: CheckpointSequenceNumber,
    ) -> IotaResult {
        let mut batch = self
            .perpetual_tables
            .executed_transactions_to_checkpoint
            .batch();
        batch.insert_batch(
            &self.perpetual_tables.executed_transactions_to_checkpoint,
            digests.iter().map(|d| (*d, (epoch, sequence))),
        )?;
        batch.write()?;
        trace!("Transactions {digests:?} finalized at checkpoint {sequence} epoch {epoch}");
        Ok(())
    }

    // Implementation of the corresponding method of `CheckpointCache` trait.
    pub(crate) fn get_transaction_perpetual_checkpoint(
        &self,
        digest: &TransactionDigest,
    ) -> IotaResult<Option<(EpochId, CheckpointSequenceNumber)>> {
        Ok(self
            .perpetual_tables
            .executed_transactions_to_checkpoint
            .get(digest)?)
    }

    // Implementation of the corresponding method of `CheckpointCache` trait.
    pub(crate) fn multi_get_transactions_perpetual_checkpoints(
        &self,
        digests: &[TransactionDigest],
    ) -> IotaResult<Vec<Option<(EpochId, CheckpointSequenceNumber)>>> {
        Ok(self
            .perpetual_tables
            .executed_transactions_to_checkpoint
            .multi_get(digests)?)
    }

    /// Returns true if there are no objects in the database
    pub fn database_is_empty(&self) -> IotaResult<bool> {
        self.perpetual_tables.database_is_empty()
    }

    /// A function that acquires all locks associated with the objects (in order
    /// to avoid deadlocks).
    async fn acquire_locks(&self, input_objects: &[ObjectRef]) -> Vec<MutexGuard> {
        self.mutex_table
            .acquire_locks(input_objects.iter().map(|(_, _, digest)| *digest))
            .await
    }

    pub fn object_exists_by_key(
        &self,
        object_id: &ObjectID,
        version: VersionNumber,
    ) -> IotaResult<bool> {
        Ok(self
            .perpetual_tables
            .objects
            .contains_key(&ObjectKey(*object_id, version))?)
    }

    pub fn multi_object_exists_by_key(&self, object_keys: &[ObjectKey]) -> IotaResult<Vec<bool>> {
        Ok(self
            .perpetual_tables
            .objects
            .multi_contains_keys(object_keys.to_vec())?
            .into_iter()
            .collect())
    }

    pub fn multi_get_objects_by_key(
        &self,
        object_keys: &[ObjectKey],
    ) -> Result<Vec<Option<Object>>, IotaError> {
        let wrappers = self
            .perpetual_tables
            .objects
            .multi_get(object_keys.to_vec())?;
        let mut ret = vec![];

        for (idx, w) in wrappers.into_iter().enumerate() {
            ret.push(
                w.map(|object| self.perpetual_tables.object(&object_keys[idx], object))
                    .transpose()?
                    .flatten(),
            );
        }
        Ok(ret)
    }

    /// Get many objects
    pub fn get_objects(&self, objects: &[ObjectID]) -> Result<Vec<Option<Object>>, IotaError> {
        let mut result = Vec::new();
        for id in objects {
            result.push(self.get_object(id)?);
        }
        Ok(result)
    }

    pub fn have_deleted_owned_object_at_version_or_after(
        &self,
        object_id: &ObjectID,
        version: VersionNumber,
        epoch_id: EpochId,
    ) -> Result<bool, IotaError> {
        let object_key = ObjectKey::max_for_id(object_id);
        let marker_key = (epoch_id, object_key);

        // Find the most recent version of the object that was deleted or wrapped.
        // Return true if the version is >= `version`. Otherwise return false.
        let marker_entry = self
            .perpetual_tables
            .object_per_epoch_marker_table
            .unbounded_iter()
            .skip_prior_to(&marker_key)?
            .next();
        match marker_entry {
            Some(((epoch, key), marker)) => {
                // Make sure object id matches and version is >= `version`
                let object_data_ok = key.0 == *object_id && key.1 >= version;
                // Make sure we don't have a stale epoch for some reason (e.g., a revert)
                let epoch_data_ok = epoch == epoch_id;
                // Make sure the object was deleted or wrapped.
                let mark_data_ok = marker == MarkerValue::OwnedDeleted;
                Ok(object_data_ok && epoch_data_ok && mark_data_ok)
            }
            None => Ok(false),
        }
    }

    // Methods to mutate the store

    /// Insert a genesis object.
    /// TODO: delete this method entirely (still used by authority_tests.rs)
    pub(crate) fn insert_genesis_object(&self, object: Object) -> IotaResult {
        // We only side load objects with a genesis parent transaction.
        debug_assert!(object.previous_transaction == TransactionDigest::genesis_marker());
        let object_ref = object.compute_object_reference();
        self.insert_object_direct(object_ref, &object)
    }

    /// Insert an object directly into the store, and also update relevant
    /// tables NOTE: does not handle transaction lock.
    /// This is used to insert genesis objects
    fn insert_object_direct(&self, object_ref: ObjectRef, object: &Object) -> IotaResult {
        let mut write_batch = self.perpetual_tables.objects.batch();

        // Insert object
        let StoreObjectPair(store_object, indirect_object) =
            get_store_object_pair(object.clone(), self.indirect_objects_threshold);
        write_batch.insert_batch(
            &self.perpetual_tables.objects,
            std::iter::once((ObjectKey::from(object_ref), store_object)),
        )?;
        if let Some(indirect_obj) = indirect_object {
            write_batch.insert_batch(
                &self.perpetual_tables.indirect_move_objects,
                std::iter::once((indirect_obj.inner().digest(), indirect_obj)),
            )?;
        }

        // Update the index
        if object.get_single_owner().is_some() {
            // Only initialize live object markers for address owned objects.
            if !object.is_child_object() {
                self.initialize_live_object_markers_impl(&mut write_batch, &[object_ref])?;
            }
        }

        write_batch.write()?;

        Ok(())
    }

    /// This function should only be used for initializing genesis and should
    /// remain private.
    #[instrument(level = "debug", skip_all)]
    pub(crate) fn bulk_insert_genesis_objects(&self, objects: &[Object]) -> IotaResult<()> {
        let mut batch = self.perpetual_tables.objects.batch();
        let ref_and_objects: Vec<_> = objects
            .iter()
            .map(|o| (o.compute_object_reference(), o))
            .collect();

        batch
            .insert_batch(
                &self.perpetual_tables.objects,
                ref_and_objects.iter().map(|(oref, o)| {
                    (
                        ObjectKey::from(oref),
                        get_store_object_pair((*o).clone(), self.indirect_objects_threshold).0,
                    )
                }),
            )?
            .insert_batch(
                &self.perpetual_tables.indirect_move_objects,
                ref_and_objects.iter().filter_map(|(_, o)| {
                    let StoreObjectPair(_, indirect_object) =
                        get_store_object_pair((*o).clone(), self.indirect_objects_threshold);
                    indirect_object.map(|obj| (obj.inner().digest(), obj))
                }),
            )?;

        let non_child_object_refs: Vec<_> = ref_and_objects
            .iter()
            .filter(|(_, object)| !object.is_child_object())
            .map(|(oref, _)| *oref)
            .collect();

        self.initialize_live_object_markers_impl(&mut batch, &non_child_object_refs)?;

        batch.write()?;

        Ok(())
    }

    pub fn bulk_insert_live_objects(
        perpetual_db: &AuthorityPerpetualTables,
        live_objects: impl Iterator<Item = LiveObject>,
        indirect_objects_threshold: usize,
        expected_sha3_digest: &[u8; 32],
    ) -> IotaResult<()> {
        let mut hasher = Sha3_256::default();
        let mut batch = perpetual_db.objects.batch();
        for object in live_objects {
            hasher.update(object.object_reference().2.inner());
            match object {
                LiveObject::Normal(object) => {
                    let StoreObjectPair(store_object_wrapper, indirect_object) =
                        get_store_object_pair(object.clone(), indirect_objects_threshold);
                    batch.insert_batch(
                        &perpetual_db.objects,
                        std::iter::once((
                            ObjectKey::from(object.compute_object_reference()),
                            store_object_wrapper,
                        )),
                    )?;
                    if let Some(indirect_object) = indirect_object {
                        batch.merge_batch(
                            &perpetual_db.indirect_move_objects,
                            iter::once((indirect_object.inner().digest(), indirect_object)),
                        )?;
                    }
                    if !object.is_child_object() {
                        Self::initialize_live_object_markers(
                            &perpetual_db.live_owned_object_markers,
                            &mut batch,
                            &[object.compute_object_reference()],
                        )?;
                    }
                }
                LiveObject::Wrapped(object_key) => {
                    batch.insert_batch(
                        &perpetual_db.objects,
                        std::iter::once::<(ObjectKey, StoreObjectWrapper)>((
                            object_key,
                            StoreObject::Wrapped.into(),
                        )),
                    )?;
                }
            }
        }
        let sha3_digest = hasher.finalize().digest;
        if *expected_sha3_digest != sha3_digest {
            error!(
                "Sha does not match! expected: {:?}, actual: {:?}",
                expected_sha3_digest, sha3_digest
            );
            return Err(IotaError::from("Sha does not match"));
        }
        batch.write()?;
        Ok(())
    }

    pub fn set_epoch_start_configuration(
        &self,
        epoch_start_configuration: &EpochStartConfiguration,
    ) -> IotaResult {
        self.perpetual_tables
            .set_epoch_start_configuration(epoch_start_configuration)?;
        Ok(())
    }

    pub fn get_epoch_start_configuration(&self) -> IotaResult<Option<EpochStartConfiguration>> {
        Ok(self.perpetual_tables.epoch_start_configuration.get(&())?)
    }

    /// Acquires read locks for affected indirect objects
    #[instrument(level = "trace", skip_all)]
    async fn acquire_read_locks_for_indirect_objects(
        &self,
        written: &[Object],
    ) -> Vec<RwLockGuard> {
        // locking is required to avoid potential race conditions with the pruner
        // potential race:
        //   - transaction execution branches to reference count increment
        //   - pruner decrements ref count to 0
        //   - compaction job compresses existing merge values to an empty vector
        //   - tx executor commits ref count increment instead of the full value making
        //     object inaccessible
        // read locks are sufficient because ref count increments are safe,
        // concurrent transaction executions produce independent ref count increments
        // and don't corrupt the state
        let digests = written
            .iter()
            .filter_map(|object| {
                let StoreObjectPair(_, indirect_object) =
                    get_store_object_pair(object.clone(), self.indirect_objects_threshold);
                indirect_object.map(|obj| obj.inner().digest())
            })
            .collect();
        self.objects_lock_table.acquire_read_locks(digests).await
    }

    /// Updates the state resulting from the execution of a certificate.
    ///
    /// Internally it checks that all locks for active inputs are at the correct
    /// version, and then writes objects, certificates, parents and clean up
    /// locks atomically.
    #[instrument(level = "debug", skip_all)]
    pub async fn write_transaction_outputs(
        &self,
        epoch_id: EpochId,
        tx_outputs: &[Arc<TransactionOutputs>],
    ) -> IotaResult {
        let mut written = Vec::with_capacity(tx_outputs.len());
        for outputs in tx_outputs {
            written.extend(outputs.written.values().cloned());
        }

        let _locks = self.acquire_read_locks_for_indirect_objects(&written).await;

        let mut write_batch = self.perpetual_tables.transactions.batch();
        for outputs in tx_outputs {
            self.write_one_transaction_outputs(&mut write_batch, epoch_id, outputs)?;
        }
        // test crashing before writing the batch
        fail_point_async!("crash");

        write_batch.write()?;
        trace!(
            "committed transactions: {:?}",
            tx_outputs
                .iter()
                .map(|tx| tx.transaction.digest())
                .collect::<Vec<_>>()
        );

        // test crashing before notifying
        fail_point_async!("crash");

        Ok(())
    }

    fn write_one_transaction_outputs(
        &self,
        write_batch: &mut DBBatch,
        epoch_id: EpochId,
        tx_outputs: &TransactionOutputs,
    ) -> IotaResult {
        let TransactionOutputs {
            transaction,
            effects,
            markers,
            wrapped,
            deleted,
            written,
            events,
            live_object_markers_to_delete,
            new_live_object_markers_to_init,
            ..
        } = tx_outputs;

        // Store the certificate indexed by transaction digest
        let transaction_digest = transaction.digest();
        write_batch.insert_batch(
            &self.perpetual_tables.transactions,
            iter::once((transaction_digest, transaction.serializable_ref())),
        )?;

        // Add batched writes for objects and locks.
        let effects_digest = effects.digest();

        write_batch.insert_batch(
            &self.perpetual_tables.object_per_epoch_marker_table,
            markers
                .iter()
                .map(|(key, marker_value)| ((epoch_id, *key), *marker_value)),
        )?;

        write_batch.insert_batch(
            &self.perpetual_tables.objects,
            deleted
                .iter()
                .map(|key| (key, StoreObject::Deleted))
                .chain(wrapped.iter().map(|key| (key, StoreObject::Wrapped)))
                .map(|(key, store_object)| (key, StoreObjectWrapper::from(store_object))),
        )?;

        // Insert each output object into the stores
        let (new_objects, new_indirect_move_objects): (Vec<_>, Vec<_>) = written
            .iter()
            .map(|(id, new_object)| {
                let version = new_object.version();
                debug!(?id, ?version, "writing object");
                let StoreObjectPair(store_object, indirect_object) =
                    get_store_object_pair(new_object.clone(), self.indirect_objects_threshold);
                (
                    (ObjectKey(*id, version), store_object),
                    indirect_object.map(|obj| (obj.inner().digest(), obj)),
                )
            })
            .unzip();

        let indirect_objects: Vec<_> = new_indirect_move_objects.into_iter().flatten().collect();
        let existing_digests = self
            .perpetual_tables
            .indirect_move_objects
            .multi_get_raw_bytes(indirect_objects.iter().map(|(digest, _)| digest))?;
        // split updates to existing and new indirect objects
        // for new objects full merge needs to be triggered. For existing ref count
        // increment is sufficient
        let (existing_indirect_objects, new_indirect_objects): (Vec<_>, Vec<_>) = indirect_objects
            .into_iter()
            .enumerate()
            .partition(|(idx, _)| matches!(&existing_digests[*idx], Some(value) if !is_ref_count_value(value)));

        write_batch.insert_batch(&self.perpetual_tables.objects, new_objects.into_iter())?;
        if !new_indirect_objects.is_empty() {
            write_batch.merge_batch(
                &self.perpetual_tables.indirect_move_objects,
                new_indirect_objects.into_iter().map(|(_, pair)| pair),
            )?;
        }
        if !existing_indirect_objects.is_empty() {
            write_batch.partial_merge_batch(
                &self.perpetual_tables.indirect_move_objects,
                existing_indirect_objects
                    .into_iter()
                    .map(|(_, (digest, _))| (digest, 1_u64.to_le_bytes())),
            )?;
        }

        let event_digest = events.digest();
        let events = events
            .data
            .iter()
            .enumerate()
            .map(|(i, e)| ((event_digest, i), e));

        write_batch.insert_batch(&self.perpetual_tables.events, events)?;

        self.initialize_live_object_markers_impl(write_batch, new_live_object_markers_to_init)?;

        // Note: deletes live object markers for received objects as well (but not for
        // objects that were in `Receiving` arguments which were not received)
        self.delete_live_object_markers(write_batch, live_object_markers_to_delete)?;

        write_batch
            .insert_batch(
                &self.perpetual_tables.effects,
                [(effects_digest, effects.clone())],
            )?
            .insert_batch(
                &self.perpetual_tables.executed_effects,
                [(transaction_digest, effects_digest)],
            )?;

        debug!(effects_digest = ?effects.digest(), "commit_certificate finished");

        Ok(())
    }

    /// Commits transactions only to the db. Called by checkpoint builder. See
    /// ExecutionCache::commit_transactions for more info
    pub(crate) fn commit_transactions(
        &self,
        transactions: &[(TransactionDigest, VerifiedTransaction)],
    ) -> IotaResult {
        let mut batch = self.perpetual_tables.transactions.batch();
        batch.insert_batch(
            &self.perpetual_tables.transactions,
            transactions
                .iter()
                .map(|(digest, tx)| (*digest, tx.serializable_ref())),
        )?;
        batch.write()?;
        Ok(())
    }

    pub async fn acquire_transaction_locks(
        &self,
        epoch_store: &AuthorityPerEpochStore,
        owned_input_objects: &[ObjectRef],
        transaction: VerifiedSignedTransaction,
    ) -> IotaResult {
        let tx_digest = *transaction.digest();
        // Other writers may be attempting to acquire locks on the same objects, so a
        // mutex is required.
        // TODO: replace with optimistic db_transactions (i.e. set lock to tx if none)
        let _mutexes = self.acquire_locks(owned_input_objects).await;

        trace!(?owned_input_objects, "acquire_transaction_locks");
        let mut locks_to_write = Vec::new();

        let live_object_markers = self
            .perpetual_tables
            .live_owned_object_markers
            .multi_get(owned_input_objects)?;

        let epoch_tables = epoch_store.tables()?;

        let locks = epoch_tables.multi_get_locked_transactions(owned_input_objects)?;

        assert_eq!(locks.len(), live_object_markers.len());

        for (live_marker, lock, obj_ref) in izip!(
            live_object_markers.into_iter(),
            locks.into_iter(),
            owned_input_objects
        ) {
            if live_marker.is_none() {
                // object at that version does not exist
                let latest_live_version = self.get_latest_live_version_for_object_id(obj_ref.0)?;
                fp_bail!(
                    UserInputError::ObjectVersionUnavailableForConsumption {
                        provided_obj_ref: *obj_ref,
                        current_version: latest_live_version.1
                    }
                    .into()
                );
            };

            if let Some(previous_tx_digest) = &lock {
                if previous_tx_digest == &tx_digest {
                    // no need to re-write lock
                    continue;
                } else {
                    // TODO: add metrics here
                    info!(prev_tx_digest = ?previous_tx_digest,
                          cur_tx_digest = ?tx_digest,
                          "Cannot acquire lock: conflicting transaction!");
                    return Err(IotaError::ObjectLockConflict {
                        obj_ref: *obj_ref,
                        pending_transaction: *previous_tx_digest,
                    });
                }
            }

            locks_to_write.push((*obj_ref, tx_digest));
        }

        if !locks_to_write.is_empty() {
            trace!(?locks_to_write, "Writing locks");
            epoch_tables.write_transaction_locks(transaction, locks_to_write.into_iter())?;
        }

        Ok(())
    }

    /// Gets ObjectLockInfo that represents state of lock on an object.
    /// Returns UserInputError::ObjectNotFound if cannot find lock record for
    /// this object
    pub(crate) fn get_lock(
        &self,
        obj_ref: ObjectRef,
        epoch_store: &AuthorityPerEpochStore,
    ) -> IotaLockResult {
        if self
            .perpetual_tables
            .live_owned_object_markers
            .get(&obj_ref)?
            .is_none()
        {
            // object at that version does not exist
            return Ok(ObjectLockStatus::LockedAtDifferentVersion {
                locked_ref: self.get_latest_live_version_for_object_id(obj_ref.0)?,
            });
        }

        let tables = epoch_store.tables()?;
        if let Some(tx_digest) = tables.get_locked_transaction(&obj_ref)? {
            Ok(ObjectLockStatus::LockedToTx {
                locked_by_tx: tx_digest,
            })
        } else {
            Ok(ObjectLockStatus::Initialized)
        }
    }

    /// Returns UserInputError::ObjectNotFound if no lock records found for this
    /// object.
    pub(crate) fn get_latest_live_version_for_object_id(
        &self,
        object_id: ObjectID,
    ) -> IotaResult<ObjectRef> {
        let mut iterator = self
            .perpetual_tables
            .live_owned_object_markers
            .unbounded_iter()
            // Make the max possible entry for this object ID.
            .skip_prior_to(&(object_id, SequenceNumber::MAX, ObjectDigest::MAX))?;
        Ok(iterator
            .next()
            .and_then(|value| {
                if value.0.0 == object_id {
                    Some(value)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                IotaError::from(UserInputError::ObjectNotFound {
                    object_id,
                    version: None,
                })
            })?
            .0)
    }

    /// Checks multiple object locks exist.
    /// Returns UserInputError::ObjectNotFound if cannot find lock record for at
    /// least one of the objects.
    /// Returns UserInputError::ObjectVersionUnavailableForConsumption if at
    /// least one object lock is not initialized     at the given version.
    pub fn check_owned_objects_are_live(&self, objects: &[ObjectRef]) -> IotaResult {
        let live_markers = self
            .perpetual_tables
            .live_owned_object_markers
            .multi_get(objects)?;
        for (live_marker, obj_ref) in live_markers.into_iter().zip(objects) {
            if live_marker.is_none() {
                // object at that version does not exist
                let latest_live_version = self.get_latest_live_version_for_object_id(obj_ref.0)?;
                fp_bail!(
                    UserInputError::ObjectVersionUnavailableForConsumption {
                        provided_obj_ref: *obj_ref,
                        current_version: latest_live_version.1
                    }
                    .into()
                );
            }
        }
        Ok(())
    }

    /// Initialize live object markers for a given list of ObjectRefs.
    fn initialize_live_object_markers_impl(
        &self,
        write_batch: &mut DBBatch,
        objects: &[ObjectRef],
    ) -> IotaResult {
        AuthorityStore::initialize_live_object_markers(
            &self.perpetual_tables.live_owned_object_markers,
            write_batch,
            objects,
        )
    }

    pub fn initialize_live_object_markers(
        live_object_marker_table: &DBMap<ObjectRef, ()>,
        write_batch: &mut DBBatch,
        objects: &[ObjectRef],
    ) -> IotaResult {
        trace!(?objects, "initialize_live_object_markers");

        write_batch.insert_batch(
            live_object_marker_table,
            objects.iter().map(|obj_ref| (obj_ref, ())),
        )?;
        Ok(())
    }

    /// Removes locks for a given list of ObjectRefs.
    fn delete_live_object_markers(
        &self,
        write_batch: &mut DBBatch,
        objects: &[ObjectRef],
    ) -> IotaResult {
        trace!(?objects, "delete_live_object_markers");
        write_batch.delete_batch(
            &self.perpetual_tables.live_owned_object_markers,
            objects.iter(),
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reset_locks_and_live_markers_for_test(
        &self,
        transactions: &[TransactionDigest],
        objects: &[ObjectRef],
        epoch_store: &AuthorityPerEpochStore,
    ) {
        for tx in transactions {
            epoch_store.delete_signed_transaction_for_test(tx);
            epoch_store.delete_object_locks_for_test(objects);
        }

        let mut batch = self.perpetual_tables.live_owned_object_markers.batch();
        batch
            .delete_batch(
                &self.perpetual_tables.live_owned_object_markers,
                objects.iter(),
            )
            .unwrap();
        batch.write().unwrap();

        let mut batch = self.perpetual_tables.live_owned_object_markers.batch();
        self.initialize_live_object_markers_impl(&mut batch, objects)
            .unwrap();
        batch.write().unwrap();
    }

    /// This function is called at the end of epoch for each transaction that's
    /// executed locally on the validator but didn't make to the last
    /// checkpoint. The effects of the execution is reverted here.
    /// The following things are reverted:
    /// 1. All new object states are deleted.
    /// 2. owner_index table change is reverted.
    ///
    /// NOTE: transaction and effects are intentionally not deleted. It's
    /// possible that if this node is behind, the network will execute the
    /// transaction in a later epoch. In that case, we need to keep it saved
    /// so that when we receive the checkpoint that includes it from state
    /// sync, we are able to execute the checkpoint.
    /// TODO: implement GC for transactions that are no longer needed.
    pub fn revert_state_update(&self, tx_digest: &TransactionDigest) -> IotaResult {
        let Some(effects) = self.get_executed_effects(tx_digest)? else {
            info!("Not reverting {:?} as it was not executed", tx_digest);
            return Ok(());
        };

        info!(?tx_digest, ?effects, "reverting transaction");

        // We should never be reverting shared object transactions.
        assert!(effects.input_shared_objects().is_empty());

        let mut write_batch = self.perpetual_tables.transactions.batch();
        write_batch.delete_batch(
            &self.perpetual_tables.executed_effects,
            iter::once(tx_digest),
        )?;
        if let Some(events_digest) = effects.events_digest() {
            write_batch.schedule_delete_range(
                &self.perpetual_tables.events,
                &(*events_digest, usize::MIN),
                &(*events_digest, usize::MAX),
            )?;
        }

        let tombstones = effects
            .all_tombstones()
            .into_iter()
            .map(|(id, version)| ObjectKey(id, version));
        write_batch.delete_batch(&self.perpetual_tables.objects, tombstones)?;

        let all_new_object_keys = effects
            .all_changed_objects()
            .into_iter()
            .map(|((id, version, _), _, _)| ObjectKey(id, version));
        write_batch.delete_batch(&self.perpetual_tables.objects, all_new_object_keys.clone())?;

        let modified_object_keys = effects
            .modified_at_versions()
            .into_iter()
            .map(|(id, version)| ObjectKey(id, version));

        macro_rules! get_objects_and_locks {
            ($object_keys: expr) => {
                self.perpetual_tables
                    .objects
                    .multi_get($object_keys.clone())?
                    .into_iter()
                    .zip($object_keys)
                    .filter_map(|(obj_opt, key)| {
                        let obj = self
                            .perpetual_tables
                            .object(
                                &key,
                                obj_opt.unwrap_or_else(|| {
                                    panic!("Older object version not found: {:?}", key)
                                }),
                            )
                            .expect("Matching indirect object not found")?;

                        if obj.is_immutable() {
                            return None;
                        }

                        let obj_ref = obj.compute_object_reference();
                        Some(obj.is_address_owned().then_some(obj_ref))
                    })
            };
        }

        let old_locks = get_objects_and_locks!(modified_object_keys);
        let new_locks = get_objects_and_locks!(all_new_object_keys);

        let old_locks: Vec<_> = old_locks.flatten().collect();

        // Re-create old live markers.
        self.initialize_live_object_markers_impl(&mut write_batch, &old_locks)?;

        // Delete new live markers
        write_batch.delete_batch(
            &self.perpetual_tables.live_owned_object_markers,
            new_locks.flatten(),
        )?;

        write_batch.write()?;

        Ok(())
    }

    /// Return the object with version less then or eq to the provided seq
    /// number. This is used by indexer to find the correct version of
    /// dynamic field child object. We do not store the version of the child
    /// object, but because of lamport timestamp, we know the child must
    /// have version number less then or eq to the parent.
    pub fn find_object_lt_or_eq_version(
        &self,
        object_id: ObjectID,
        version: SequenceNumber,
    ) -> IotaResult<Option<Object>> {
        self.perpetual_tables
            .find_object_lt_or_eq_version(object_id, version)
    }

    /// Returns the latest object reference we have for this object_id in the
    /// objects table.
    ///
    /// The method may also return the reference to a deleted object with a
    /// digest of ObjectDigest::deleted() or ObjectDigest::wrapped() and
    /// lamport version of a transaction that deleted the object.
    /// Note that a deleted object may re-appear if the deletion was the result
    /// of the object being wrapped in another object.
    ///
    /// If no entry for the object_id is found, return None.
    pub fn get_latest_object_ref_or_tombstone(
        &self,
        object_id: ObjectID,
    ) -> Result<Option<ObjectRef>, IotaError> {
        self.perpetual_tables
            .get_latest_object_ref_or_tombstone(object_id)
    }

    /// Returns the latest object reference if and only if the object is still
    /// live (i.e. it does not return tombstones)
    pub fn get_latest_object_ref_if_alive(
        &self,
        object_id: ObjectID,
    ) -> Result<Option<ObjectRef>, IotaError> {
        match self.get_latest_object_ref_or_tombstone(object_id)? {
            Some(objref) if objref.2.is_alive() => Ok(Some(objref)),
            _ => Ok(None),
        }
    }

    /// Returns the latest object we have for this object_id in the objects
    /// table.
    ///
    /// If no entry for the object_id is found, return None.
    pub fn get_latest_object_or_tombstone(
        &self,
        object_id: ObjectID,
    ) -> Result<Option<(ObjectKey, ObjectOrTombstone)>, IotaError> {
        let Some((object_key, store_object)) = self
            .perpetual_tables
            .get_latest_object_or_tombstone(object_id)?
        else {
            return Ok(None);
        };

        if let Some(object_ref) = self
            .perpetual_tables
            .tombstone_reference(&object_key, &store_object)?
        {
            return Ok(Some((object_key, ObjectOrTombstone::Tombstone(object_ref))));
        }

        let object = self
            .perpetual_tables
            .object(&object_key, store_object)?
            .expect("Non tombstone store object could not be converted to object");

        Ok(Some((object_key, ObjectOrTombstone::Object(object))))
    }

    pub fn insert_transaction_and_effects(
        &self,
        transaction: &VerifiedTransaction,
        transaction_effects: &TransactionEffects,
    ) -> Result<(), TypedStoreError> {
        let mut write_batch = self.perpetual_tables.transactions.batch();
        write_batch
            .insert_batch(
                &self.perpetual_tables.transactions,
                [(transaction.digest(), transaction.serializable_ref())],
            )?
            .insert_batch(
                &self.perpetual_tables.effects,
                [(transaction_effects.digest(), transaction_effects)],
            )?;

        write_batch.write()?;
        Ok(())
    }

    pub fn multi_insert_transaction_and_effects<'a>(
        &self,
        transactions: impl Iterator<Item = &'a VerifiedExecutionData>,
    ) -> Result<(), TypedStoreError> {
        let mut write_batch = self.perpetual_tables.transactions.batch();
        for tx in transactions {
            write_batch
                .insert_batch(
                    &self.perpetual_tables.transactions,
                    [(tx.transaction.digest(), tx.transaction.serializable_ref())],
                )?
                .insert_batch(
                    &self.perpetual_tables.effects,
                    [(tx.effects.digest(), &tx.effects)],
                )?;
        }

        write_batch.write()?;
        Ok(())
    }

    pub fn multi_get_transaction_blocks(
        &self,
        tx_digests: &[TransactionDigest],
    ) -> IotaResult<Vec<Option<VerifiedTransaction>>> {
        Ok(self
            .perpetual_tables
            .transactions
            .multi_get(tx_digests)
            .map(|v| v.into_iter().map(|v| v.map(|v| v.into())).collect())?)
    }

    pub fn get_transaction_block(
        &self,
        tx_digest: &TransactionDigest,
    ) -> Result<Option<VerifiedTransaction>, TypedStoreError> {
        self.perpetual_tables
            .transactions
            .get(tx_digest)
            .map(|v| v.map(|v| v.into()))
    }

    /// This function reads the DB directly to get the system state object.
    /// If reconfiguration is happening at the same time, there is no guarantee
    /// whether we would be getting the old or the new system state object.
    /// Hence this function should only be called during RPC reads where data
    /// race is not a major concern. In general we should avoid this as much
    /// as possible. If the intent is for testing, you can use
    /// AuthorityState:: get_iota_system_state_object_for_testing.
    pub fn get_iota_system_state_object_unsafe(&self) -> IotaResult<IotaSystemState> {
        get_iota_system_state(self.perpetual_tables.as_ref())
    }

    pub fn expensive_check_iota_conservation<T>(
        self: &Arc<Self>,
        type_layout_store: T,
        old_epoch_store: &AuthorityPerEpochStore,
        epoch_supply_change: Option<i64>,
    ) -> IotaResult
    where
        T: TypeLayoutStore + Send + Copy,
    {
        if !self.enable_epoch_iota_conservation_check {
            return Ok(());
        }

        let executor = old_epoch_store.executor();
        info!("Starting IOTA conservation check. This may take a while..");
        let cur_time = Instant::now();
        let mut pending_objects = vec![];
        let mut count = 0;
        let mut size = 0;
        let (mut total_iota, mut total_storage_rebate) = thread::scope(|s| {
            let pending_tasks = FuturesUnordered::new();
            for o in self.iter_live_object_set() {
                match o {
                    LiveObject::Normal(object) => {
                        size += object.object_size_for_gas_metering();
                        count += 1;
                        pending_objects.push(object);
                        if count % 1_000_000 == 0 {
                            let mut task_objects = vec![];
                            mem::swap(&mut pending_objects, &mut task_objects);
                            pending_tasks.push(s.spawn(move || {
                                let mut layout_resolver =
                                    executor.type_layout_resolver(Box::new(type_layout_store));
                                let mut total_storage_rebate = 0;
                                let mut total_iota = 0;
                                for object in task_objects {
                                    total_storage_rebate += object.storage_rebate;
                                    // get_total_iota includes storage rebate, however all storage
                                    // rebate is also stored in
                                    // the storage fund, so we need to subtract it here.
                                    total_iota +=
                                        object.get_total_iota(layout_resolver.as_mut()).unwrap()
                                            - object.storage_rebate;
                                }
                                if count % 50_000_000 == 0 {
                                    info!("Processed {} objects", count);
                                }
                                (total_iota, total_storage_rebate)
                            }));
                        }
                    }
                    LiveObject::Wrapped(_) => {
                        unreachable!("Explicitly asked to not include wrapped tombstones")
                    }
                }
            }
            pending_tasks.into_iter().fold((0, 0), |init, result| {
                let result = result.join().unwrap();
                (init.0 + result.0, init.1 + result.1)
            })
        });
        let mut layout_resolver = executor.type_layout_resolver(Box::new(type_layout_store));
        for object in pending_objects {
            total_storage_rebate += object.storage_rebate;
            total_iota +=
                object.get_total_iota(layout_resolver.as_mut()).unwrap() - object.storage_rebate;
        }
        info!(
            "Scanned {} live objects, took {:?}",
            count,
            cur_time.elapsed()
        );
        self.metrics
            .iota_conservation_live_object_count
            .set(count as i64);
        self.metrics
            .iota_conservation_live_object_size
            .set(size as i64);
        self.metrics
            .iota_conservation_check_latency
            .set(cur_time.elapsed().as_secs() as i64);

        // It is safe to call this function because we are in the middle of
        // reconfiguration.
        let system_state: IotaSystemStateSummaryV2 = self
            .get_iota_system_state_object_unsafe()
            .expect("Reading iota system state object cannot fail")
            .into_iota_system_state_summary()
            .try_into()?;
        let storage_fund_balance = system_state.storage_fund_total_object_storage_rebates;
        info!(
            "Total IOTA amount in the network: {}, storage fund balance: {}, total storage rebate: {} at beginning of epoch {}",
            total_iota, storage_fund_balance, total_storage_rebate, system_state.epoch
        );

        let imbalance = (storage_fund_balance as i64) - (total_storage_rebate as i64);
        self.metrics
            .iota_conservation_storage_fund
            .set(storage_fund_balance as i64);
        self.metrics
            .iota_conservation_storage_fund_imbalance
            .set(imbalance);
        self.metrics
            .iota_conservation_imbalance
            .set((total_iota as i128 - system_state.iota_total_supply as i128) as i64);

        if let Some(expected_imbalance) = self
            .perpetual_tables
            .expected_storage_fund_imbalance
            .get(&())
            .map_err(|err| {
                IotaError::from(
                    format!("failed to read expected storage fund imbalance: {err}").as_str(),
                )
            })?
        {
            fp_ensure!(
                imbalance == expected_imbalance,
                IotaError::from(
                    format!(
                        "Inconsistent state detected at epoch {}: total storage rebate: {}, storage fund balance: {}, expected imbalance: {}",
                        system_state.epoch, total_storage_rebate, storage_fund_balance, expected_imbalance
                    ).as_str()
                )
            );
        } else {
            self.perpetual_tables
                .expected_storage_fund_imbalance
                .insert(&(), &imbalance)
                .map_err(|err| {
                    IotaError::from(
                        format!("failed to write expected storage fund imbalance: {err}").as_str(),
                    )
                })?;
        }

        let total_supply = self
            .perpetual_tables
            .total_iota_supply
            .get(&())
            .map_err(|err| {
                IotaError::from(format!("failed to read total iota supply: {err}").as_str())
            })?;

        match total_supply.zip(epoch_supply_change) {
            // Only execute the check if both are set and the supply value was set in the last
            // epoch. We have to assume the supply changes every epoch and therefore we
            // cannot run the check with a supply value from any epoch earlier than the
            // last one. This can happen if the check was disabled for some time.
            Some((old_supply, epoch_supply_change))
                if old_supply.last_check_epoch + 1 == old_epoch_store.epoch() =>
            {
                let expected_new_supply = if epoch_supply_change >= 0 {
                    old_supply
                        .total_supply
                        .checked_add(epoch_supply_change.unsigned_abs())
                        .ok_or_else(|| {
                            IotaError::from(
                                format!(
                                    "Inconsistent state detected at epoch {}: old supply {} + supply change {} overflowed",
                                    system_state.epoch, old_supply.total_supply, epoch_supply_change
                                ).as_str())
                        })?
                } else {
                    old_supply.total_supply.checked_sub(epoch_supply_change.unsigned_abs()).ok_or_else(|| {
                        IotaError::from(
                            format!(
                                "Inconsistent state detected at epoch {}: old supply {} - supply change {} underflowed",
                                system_state.epoch, old_supply.total_supply, epoch_supply_change
                            ).as_str())
                    })?
                };

                fp_ensure!(
                    total_iota == expected_new_supply,
                    IotaError::from(
                        format!(
                            "Inconsistent state detected at epoch {}: total iota: {}, expecting {}",
                            system_state.epoch, total_iota, expected_new_supply
                        )
                        .as_str()
                    )
                );

                let new_supply = TotalIotaSupplyCheck {
                    total_supply: expected_new_supply,
                    last_check_epoch: old_epoch_store.epoch(),
                };

                self.perpetual_tables
                    .total_iota_supply
                    .insert(&(), &new_supply)
                    .map_err(|err| {
                        IotaError::from(
                            format!("failed to write total iota supply: {err}").as_str(),
                        )
                    })?;
            }
            // If either one is None or if the last value is from an older epoch,
            // we update the value in the table since we're at genesis and cannot execute the check.
            _ => {
                info!("Skipping total supply check");

                let supply = TotalIotaSupplyCheck {
                    total_supply: total_iota,
                    last_check_epoch: old_epoch_store.epoch(),
                };

                self.perpetual_tables
                    .total_iota_supply
                    .insert(&(), &supply)
                    .map_err(|err| {
                        IotaError::from(
                            format!("failed to write total iota supply: {err}").as_str(),
                        )
                    })?;

                return Ok(());
            }
        };

        Ok(())
    }

    pub async fn prune_objects_and_compact_for_testing(
        &self,
        checkpoint_store: &Arc<CheckpointStore>,
        rest_index: Option<&RestIndexStore>,
    ) {
        let pruning_config = AuthorityStorePruningConfig {
            num_epochs_to_retain: 0,
            ..Default::default()
        };
        let _ = AuthorityStorePruner::prune_objects_for_eligible_epochs(
            &self.perpetual_tables,
            checkpoint_store,
            rest_index,
            &self.objects_lock_table,
            pruning_config,
            AuthorityStorePruningMetrics::new_for_test(),
            usize::MAX,
            EPOCH_DURATION_MS_FOR_TESTING,
        )
        .await;
        let _ = AuthorityStorePruner::compact(&self.perpetual_tables);
    }

    #[cfg(test)]
    pub async fn prune_objects_immediately_for_testing(
        &self,
        transaction_effects: Vec<TransactionEffects>,
    ) -> anyhow::Result<()> {
        let mut wb = self.perpetual_tables.objects.batch();

        let mut object_keys_to_prune = vec![];
        for effects in &transaction_effects {
            for (object_id, seq_number) in effects.modified_at_versions() {
                info!("Pruning object {:?} version {:?}", object_id, seq_number);
                object_keys_to_prune.push(ObjectKey(object_id, seq_number));
            }
        }

        wb.delete_batch(
            &self.perpetual_tables.objects,
            object_keys_to_prune.into_iter(),
        )?;
        wb.write()?;
        Ok(())
    }

    #[cfg(msim)]
    pub fn remove_all_versions_of_object(&self, object_id: ObjectID) {
        let entries: Vec<_> = self
            .perpetual_tables
            .objects
            .unbounded_iter()
            .filter_map(|(key, _)| if key.0 == object_id { Some(key) } else { None })
            .collect();
        info!("Removing all versions of object: {:?}", entries);
        self.perpetual_tables.objects.multi_remove(entries).unwrap();
    }

    // Counts the number of versions exist in object store for `object_id`. This
    // includes tombstone.
    #[cfg(msim)]
    pub fn count_object_versions(&self, object_id: ObjectID) -> usize {
        self.perpetual_tables
            .objects
            .safe_iter_with_bounds(
                Some(ObjectKey(object_id, VersionNumber::MIN)),
                Some(ObjectKey(object_id, VersionNumber::MAX)),
            )
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len()
    }
}

impl AccumulatorStore for AuthorityStore {
    fn get_root_state_accumulator_for_epoch(
        &self,
        epoch: EpochId,
    ) -> IotaResult<Option<(CheckpointSequenceNumber, Accumulator)>> {
        self.perpetual_tables
            .root_state_hash_by_epoch
            .get(&epoch)
            .map_err(Into::into)
    }

    fn get_root_state_accumulator_for_highest_epoch(
        &self,
    ) -> IotaResult<Option<(EpochId, (CheckpointSequenceNumber, Accumulator))>> {
        Ok(self
            .perpetual_tables
            .root_state_hash_by_epoch
            .safe_iter()
            .skip_to_last()
            .next()
            .transpose()?)
    }

    fn insert_state_accumulator_for_epoch(
        &self,
        epoch: EpochId,
        last_checkpoint_of_epoch: &CheckpointSequenceNumber,
        acc: &Accumulator,
    ) -> IotaResult {
        self.perpetual_tables
            .root_state_hash_by_epoch
            .insert(&epoch, &(*last_checkpoint_of_epoch, acc.clone()))?;
        self.root_state_notify_read
            .notify(&epoch, &(*last_checkpoint_of_epoch, acc.clone()));

        Ok(())
    }

    fn iter_live_object_set(&self) -> Box<dyn Iterator<Item = LiveObject> + '_> {
        Box::new(self.perpetual_tables.iter_live_object_set())
    }
}

impl ObjectStore for AuthorityStore {
    /// Read an object and return it, or Ok(None) if the object was not found.
    fn get_object(
        &self,
        object_id: &ObjectID,
    ) -> Result<Option<Object>, iota_types::storage::error::Error> {
        self.perpetual_tables.as_ref().get_object(object_id)
    }

    fn get_object_by_key(
        &self,
        object_id: &ObjectID,
        version: VersionNumber,
    ) -> Result<Option<Object>, iota_types::storage::error::Error> {
        self.perpetual_tables.get_object_by_key(object_id, version)
    }
}

/// A wrapper to make Orphan Rule happy
pub struct ResolverWrapper {
    pub resolver: Arc<dyn BackingPackageStore + Send + Sync>,
    pub metrics: Arc<ResolverMetrics>,
}

impl ResolverWrapper {
    pub fn new(
        resolver: Arc<dyn BackingPackageStore + Send + Sync>,
        metrics: Arc<ResolverMetrics>,
    ) -> Self {
        metrics.module_cache_size.set(0);
        ResolverWrapper { resolver, metrics }
    }

    fn inc_cache_size_gauge(&self) {
        // reset the gauge after a restart of the cache
        let current = self.metrics.module_cache_size.get();
        self.metrics.module_cache_size.set(current + 1);
    }
}

impl ModuleResolver for ResolverWrapper {
    type Error = IotaError;
    fn get_module(&self, module_id: &ModuleId) -> Result<Option<Vec<u8>>, Self::Error> {
        self.inc_cache_size_gauge();
        get_module(&*self.resolver, module_id)
    }
}

pub enum UpdateType {
    Transaction(TransactionEffectsDigest),
    Genesis,
}

pub type IotaLockResult = IotaResult<ObjectLockStatus>;

#[derive(Debug, PartialEq, Eq)]
pub enum ObjectLockStatus {
    Initialized,
    LockedToTx { locked_by_tx: LockDetails }, // no need to use wrapper, not stored or serialized
    LockedAtDifferentVersion { locked_ref: ObjectRef },
}
