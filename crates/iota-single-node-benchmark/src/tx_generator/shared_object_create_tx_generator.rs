// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use iota_test_transaction_builder::TestTransactionBuilder;
use iota_types::{
    base_types::ObjectID,
    transaction::{DEFAULT_VALIDATOR_GAS_PRICE, Transaction},
};

use crate::{mock_account::Account, tx_generator::TxGenerator};

pub struct SharedObjectCreateTxGenerator {
    move_package: ObjectID,
}

impl SharedObjectCreateTxGenerator {
    pub fn new(move_package: ObjectID) -> Self {
        Self { move_package }
    }
}

impl TxGenerator for SharedObjectCreateTxGenerator {
    fn generate_tx(&self, account: Account) -> Transaction {
        TestTransactionBuilder::new(
            account.sender,
            account.gas_objects[0],
            DEFAULT_VALIDATOR_GAS_PRICE,
        )
        .move_call(
            self.move_package,
            "benchmark",
            "create_shared_counter",
            vec![],
        )
        .build_and_sign(account.keypair.as_ref())
    }

    fn name(&self) -> &'static str {
        "Shared Object Creation Transaction Generator"
    }
}
