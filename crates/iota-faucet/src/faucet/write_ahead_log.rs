// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use iota_types::{
    base_types::{IotaAddress, ObjectID},
    transaction::TransactionData,
};
use serde::{Deserialize, Serialize};
use tracing::info;
use typed_store::{
    DBMapUtils, Map, TypedStoreError,
    rocks::DBMap,
    traits::{TableSummary, TypedStoreDebug},
};
use uuid::Uuid;

/// Persistent log of transactions paying out iota from the faucet, keyed by the
/// coin serving the request.  Transactions are expected to be written to the
/// log before they are sent to full-node, and removed after receiving a
/// response back, before the coin becomes available for subsequent writes.
///
/// This allows the faucet to go down and back up, and not forget which requests
/// were in-flight that it needs to confirm succeeded or failed.
#[derive(DBMapUtils, Clone)]
pub struct WriteAheadLog {
    pub log: DBMap<ObjectID, Entry>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Entry {
    pub uuid: uuid::Bytes,
    // TODO (jian): remove recipient
    pub recipient: IotaAddress,
    pub tx: TransactionData,
    pub retry_count: u64,
    pub in_flight: bool,
}

impl WriteAheadLog {
    pub(crate) fn open(path: &Path) -> Self {
        Self::open_tables_read_write(
            path.to_path_buf(),
            typed_store::rocks::MetricConf::new("faucet_write_ahead_log"),
            None,
            None,
        )
    }

    /// Mark `coin` as reserved for transaction `tx` sending coin to
    /// `recipient`. Fails if `coin` is already in the WAL pointing to an
    /// existing transaction.
    pub(crate) fn reserve(
        &mut self,
        uuid: Uuid,
        coin: ObjectID,
        recipient: IotaAddress,
        tx: TransactionData,
    ) -> Result<(), TypedStoreError> {
        if self.log.contains_key(&coin)? {
            // Don't permit multiple writes against the same coin
            // TODO: Use a better error type than `TypedStoreError`.
            return Err(TypedStoreError::Serialization(format!(
                "Duplicate WAL entry for coin {coin:?}",
            )));
        }

        let uuid = *uuid.as_bytes();
        self.log.insert(
            &coin,
            &Entry {
                uuid,
                recipient,
                tx,
                retry_count: 0,
                in_flight: true,
            },
        )
    }

    /// Check whether `coin` has a pending transaction in the WAL.  Returns
    /// `Ok(Some(entry))` if a pending transaction exists, `Ok(None)` if
    /// not, and `Err(_)` if there was an internal error accessing the WAL.
    pub(crate) fn reclaim(&self, coin: ObjectID) -> Result<Option<Entry>, TypedStoreError> {
        match self.log.get(&coin) {
            Ok(entry) => Ok(entry),
            Err(TypedStoreError::Serialization(_)) => {
                // Remove bad log from the store, so we don't crash on start up, this can happen
                // if we update the WAL Entry and have some leftover Entry from
                // the WAL.
                self.log
                    .remove(&coin)
                    .unwrap_or_else(|_| panic!("Coin: {coin:?} unable to be removed from log."));
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    /// Indicate that the transaction in flight for `coin` has landed, and the
    /// entry in the WAL can be removed.
    pub(crate) fn commit(&mut self, coin: ObjectID) -> Result<(), TypedStoreError> {
        self.log.remove(&coin)
    }

    pub(crate) fn increment_retry_count(&mut self, coin: ObjectID) -> Result<(), TypedStoreError> {
        if let Some(mut entry) = self.log.get(&coin)? {
            entry.retry_count += 1;
            self.log.insert(&coin, &entry)?;
        }
        Ok(())
    }

    pub(crate) fn set_in_flight(
        &mut self,
        coin: ObjectID,
        bool: bool,
    ) -> Result<(), TypedStoreError> {
        if let Some(mut entry) = self.log.get(&coin)? {
            entry.in_flight = bool;
            self.log.insert(&coin, &entry)?;
        } else {
            info!(
                ?coin,
                "Attempted to set inflight a coin that was not in the WAL."
            );

            return Err(TypedStoreError::RocksDB(format!(
                "Coin object {coin:?} not found in WAL."
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use iota_types::{
        base_types::{ObjectRef, random_object_ref},
        transaction::TEST_ONLY_GAS_UNIT_FOR_TRANSFER,
    };

    use super::*;

    #[tokio::test]
    async fn reserve_reclaim_reclaim() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(&tmp.path().join("wal"));

        let uuid = Uuid::new_v4();
        let coin = random_object_ref();
        let (recv, tx) = random_request(coin);

        assert!(wal.reserve(uuid, coin.0, recv, tx.clone()).is_ok());

        // Reclaim once
        let Some(entry) = wal.reclaim(coin.0).unwrap() else {
            panic!("Entry not found for {}", coin.0);
        };

        assert_eq!(uuid, Uuid::from_bytes(entry.uuid));
        assert_eq!(recv, entry.recipient);
        assert_eq!(tx, entry.tx);

        // Reclaim again, should still be there.
        let Some(entry) = wal.reclaim(coin.0).unwrap() else {
            panic!("Entry not found for {}", coin.0);
        };

        assert_eq!(uuid, Uuid::from_bytes(entry.uuid));
        assert_eq!(recv, entry.recipient);
        assert_eq!(tx, entry.tx);
    }

    #[tokio::test]
    async fn test_increment_wal() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(&tmp.path().join("wal"));
        let uuid = Uuid::new_v4();
        let coin = random_object_ref();
        let (recv0, tx0) = random_request(coin);

        // First write goes through
        wal.reserve(uuid, coin.0, recv0, tx0).unwrap();
        wal.increment_retry_count(coin.0).unwrap();

        let entry = wal.reclaim(coin.0).unwrap().unwrap();
        assert_eq!(entry.retry_count, 1);
    }

    #[tokio::test]
    async fn reserve_reserve() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(&tmp.path().join("wal"));

        let uuid = Uuid::new_v4();
        let coin = random_object_ref();
        let (recv0, tx0) = random_request(coin);
        let (recv1, tx1) = random_request(coin);

        // First write goes through
        wal.reserve(uuid, coin.0, recv0, tx0).unwrap();

        // Second write fails because it tries to write to the same coin
        assert!(matches!(
            wal.reserve(uuid, coin.0, recv1, tx1),
            Err(TypedStoreError::Serialization(_)),
        ));
    }

    #[tokio::test]
    async fn reserve_reclaim_commit_reclaim() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(&tmp.path().join("wal"));

        let uuid = Uuid::new_v4();
        let coin = random_object_ref();
        let (recv, tx) = random_request(coin);

        wal.reserve(uuid, coin.0, recv, tx.clone()).unwrap();

        // Reclaim to show that the entry is there
        let Some(entry) = wal.reclaim(coin.0).unwrap() else {
            panic!("Entry not found for {}", coin.0);
        };

        assert_eq!(uuid, Uuid::from_bytes(entry.uuid));
        assert_eq!(recv, entry.recipient);
        assert_eq!(tx, entry.tx);

        // Commit the transaction, which removes it from the log.
        wal.commit(coin.0).unwrap();

        // Expect it to now be gone
        assert_eq!(Ok(None), wal.reclaim(coin.0));
    }

    #[tokio::test]
    async fn reserve_commit_reserve() {
        let tmp = tempfile::tempdir().unwrap();
        let mut wal = WriteAheadLog::open(&tmp.path().join("wal"));

        let uuid = Uuid::new_v4();
        let coin = random_object_ref();
        let (recv0, tx0) = random_request(coin);
        let (recv1, tx1) = random_request(coin);

        // Write the transaction
        wal.reserve(uuid, coin.0, recv0, tx0).unwrap();

        // Commit the transaction, which removes it from the log.
        wal.commit(coin.0).unwrap();

        // Write a fresh transaction, which should now pass
        wal.reserve(uuid, coin.0, recv1, tx1).unwrap();
    }

    fn random_request(coin: ObjectRef) -> (IotaAddress, TransactionData) {
        let gas_price = 1;
        let send = IotaAddress::random_for_testing_only();
        let recv = IotaAddress::random_for_testing_only();
        (
            recv,
            TransactionData::new_pay_iota(
                send,
                vec![coin],
                vec![recv],
                vec![1000],
                coin,
                gas_price * TEST_ONLY_GAS_UNIT_FOR_TRANSFER,
                gas_price,
            )
            .unwrap(),
        )
    }
}
