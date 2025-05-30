// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use iota_json_rpc_types::{IotaExecutionStatus, IotaTransactionBlockEffectsAPI};
use iota_sdk::wallet_context::WalletContext;
use iota_test_transaction_builder::{increment_counter, publish_basics_package_and_make_counter};
use iota_types::object::Owner;
use tracing::info;

use crate::{TestCaseImpl, TestContext, helper::ObjectChecker};

pub struct SharedCounterTest;

#[async_trait]
impl TestCaseImpl for SharedCounterTest {
    fn name(&self) -> &'static str {
        "SharedCounter"
    }

    fn description(&self) -> &'static str {
        "Test publishing basics packages and incrementing Counter (shared object)"
    }

    async fn run(&self, ctx: &mut TestContext) -> Result<(), anyhow::Error> {
        info!("Testing shared object transactions.");

        let iota_objs = ctx.get_iota_from_faucet(Some(1)).await;
        assert!(!iota_objs.is_empty());

        let wallet_context: &WalletContext = ctx.get_wallet();
        let address = ctx.get_wallet_address();
        let (package_ref, (counter_id, initial_counter_version, _)) =
            publish_basics_package_and_make_counter(wallet_context).await;
        let response = increment_counter(
            wallet_context,
            address,
            None,
            package_ref.0,
            counter_id,
            initial_counter_version,
        )
        .await;
        assert_eq!(
            *response.effects.as_ref().unwrap().status(),
            IotaExecutionStatus::Success,
            "Increment counter txn failed: {:?}",
            *response.effects.as_ref().unwrap().status()
        );

        response
            .effects
            .as_ref()
            .unwrap()
            .shared_objects()
            .iter()
            .find(|o| o.object_id == counter_id)
            .unwrap_or_else(|| panic!("Expect obj {counter_id} in shared_objects"));

        let counter_version = response
            .effects
            .as_ref()
            .unwrap()
            .mutated()
            .iter()
            .find_map(|obj| {
                let Owner::Shared {
                    initial_shared_version,
                } = obj.owner
                else {
                    return None;
                };

                if obj.reference.object_id == counter_id
                    && initial_shared_version == initial_counter_version
                {
                    Some(obj.reference.version)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("Expect obj {counter_id} in mutated"));

        // Verify fullnode observes the txn
        ctx.let_fullnode_sync(vec![response.digest], 5).await;

        let counter_object = ObjectChecker::new(counter_id)
            .owner(Owner::Shared {
                initial_shared_version: initial_counter_version,
            })
            .check_into_object(ctx.get_fullnode_client())
            .await;

        assert_eq!(
            counter_object.version, counter_version,
            "Expect sequence number to be 2"
        );

        Ok(())
    }
}
