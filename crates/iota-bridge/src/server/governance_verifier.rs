// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use crate::{
    error::{BridgeError, BridgeResult},
    server::handler::ActionVerifier,
    types::{BridgeAction, BridgeActionDigest},
};

#[derive(Debug)]
pub struct GovernanceVerifier {
    approved_governance_actions: HashMap<BridgeActionDigest, BridgeAction>,
}

impl GovernanceVerifier {
    pub fn new(approved_actions: Vec<BridgeAction>) -> BridgeResult<Self> {
        // TODO(audit-blocking): verify chain ids
        let mut approved_governance_actions = HashMap::new();
        for action in approved_actions {
            if !action.is_governace_action() {
                return Err(BridgeError::ActionIsNotGovernanceAction(action));
            }
            approved_governance_actions.insert(action.digest(), action);
        }
        Ok(Self {
            approved_governance_actions,
        })
    }
}

#[async_trait::async_trait]
impl ActionVerifier<BridgeAction> for GovernanceVerifier {
    fn name(&self) -> &'static str {
        "GovernanceVerifier"
    }

    async fn verify(&self, key: BridgeAction) -> BridgeResult<BridgeAction> {
        // TODO: an optimization would be to check the current nonce on chain and err
        // for older ones
        if !key.is_governace_action() {
            return Err(BridgeError::ActionIsNotGovernanceAction(key));
        }
        if let Some(approved_action) = self.approved_governance_actions.get(&key.digest()) {
            assert_eq!(
                &key, approved_action,
                "Mismatched action found in approved_actions"
            );
            return Ok(key);
        }
        return Err(BridgeError::GovernanceActionIsNotApproved);
    }
}

#[cfg(test)]
mod tests {
    use iota_types::bridge::BridgeChainId;

    use super::*;
    use crate::{
        test_utils::get_test_iota_to_eth_bridge_action,
        types::{BridgeAction, EmergencyAction, EmergencyActionType, LimitUpdateAction},
    };

    #[tokio::test]
    #[ignore = "https://github.com/iotaledger/iota/issues/3224"]
    async fn test_governance_verifier() {
        let action_1 = BridgeAction::EmergencyAction(EmergencyAction {
            chain_id: BridgeChainId::EthCustom,
            nonce: 1,
            action_type: EmergencyActionType::Pause,
        });
        let action_2 = BridgeAction::LimitUpdateAction(LimitUpdateAction {
            chain_id: BridgeChainId::EthCustom,
            sending_chain_id: BridgeChainId::IotaCustom,
            nonce: 1,
            new_usd_limit: 10000,
        });

        let verifier = GovernanceVerifier::new(vec![action_1.clone(), action_2.clone()]).unwrap();
        assert_eq!(
            verifier.verify(action_1.clone()).await.unwrap(),
            action_1.clone()
        );
        assert_eq!(
            verifier.verify(action_2.clone()).await.unwrap(),
            action_2.clone()
        );

        let action_3 = BridgeAction::LimitUpdateAction(LimitUpdateAction {
            chain_id: BridgeChainId::EthCustom,
            sending_chain_id: BridgeChainId::IotaCustom,
            nonce: 2,
            new_usd_limit: 10000,
        });
        assert_eq!(
            verifier.verify(action_3).await.unwrap_err(),
            BridgeError::GovernanceActionIsNotApproved
        );

        // Token transfer action is not allowed
        let action_4 = get_test_iota_to_eth_bridge_action(None, None, None, None, None, None, None);
        assert!(matches!(
            GovernanceVerifier::new(vec![action_1, action_2, action_4.clone()]).unwrap_err(),
            BridgeError::ActionIsNotGovernanceAction(..)
        ));

        // Token transfer action will be rejected
        assert!(matches!(
            verifier.verify(action_4).await.unwrap_err(),
            BridgeError::ActionIsNotGovernanceAction(..)
        ));
    }
}
