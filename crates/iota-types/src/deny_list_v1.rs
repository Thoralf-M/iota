// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use move_core_types::{
    ident_str,
    identifier::IdentStr,
    language_storage::{StructTag, TypeTag},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::error;

use crate::{
    IOTA_DENY_LIST_OBJECT_ID, IOTA_FRAMEWORK_PACKAGE_ID, MoveTypeTagTrait,
    base_types::{EpochId, IotaAddress, ObjectID, SequenceNumber},
    config::{Config, Setting},
    dynamic_field::{DOFWrapper, get_dynamic_field_from_store},
    error::{
        ExecutionError, ExecutionErrorKind, IotaError, IotaResult, UserInputError, UserInputResult,
    },
    id::{ID, UID},
    object::{Object, Owner},
    storage::{DenyListResult, ObjectStore},
    transaction::{CheckedInputObjects, ReceivingObjects},
};

pub const DENY_LIST_MODULE: &IdentStr = ident_str!("deny_list");
pub const DENY_LIST_CREATE_FUNC: &IdentStr = ident_str!("create");

pub const DENY_LIST_COIN_TYPE_INDEX: u64 = 0;

pub const CONFIG_SETTING_DYNAMIC_FIELD_SIZE_FOR_GAS: usize = 1000;

/// Rust representation of the Move type 0x2::coin::RegulatedCoinMetadata.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegulatedCoinMetadata {
    pub id: UID,
    pub coin_metadata_object: ID,
    pub deny_cap_object: ID,
}

/// Rust representation of the Move type 0x2::coin::DenyCapV1.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DenyCapV1 {
    pub id: UID,
    pub allow_global_pause: bool,
}

/// Rust representation of the Move type 0x2::deny_list::ConfigKey.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ConfigKey {
    per_type_index: u64,
    per_type_key: Vec<u8>,
}

impl ConfigKey {
    pub fn type_() -> StructTag {
        StructTag {
            address: IOTA_FRAMEWORK_PACKAGE_ID.into(),
            module: DENY_LIST_MODULE.to_owned(),
            name: ident_str!("ConfigKey").to_owned(),
            type_params: vec![],
        }
    }
}

impl MoveTypeTagTrait for ConfigKey {
    fn get_type_tag() -> TypeTag {
        TypeTag::Struct(Box::new(Self::type_()))
    }
}

/// Rust representation of the Move type 0x2::deny_list::AddressKey.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct AddressKey(IotaAddress);

impl AddressKey {
    pub fn type_() -> StructTag {
        StructTag {
            address: IOTA_FRAMEWORK_PACKAGE_ID.into(),
            module: DENY_LIST_MODULE.to_owned(),
            name: ident_str!("AddressKey").to_owned(),
            type_params: vec![],
        }
    }
}

impl MoveTypeTagTrait for AddressKey {
    fn get_type_tag() -> TypeTag {
        TypeTag::Struct(Box::new(Self::type_()))
    }
}

/// Rust representation of the Move type 0x2::deny_list::GlobalPauseKey.
/// There is no u8 in the Move definition, however empty structs in Move
/// are represented as a single byte 0 in the serialized data.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct GlobalPauseKey(bool);

impl GlobalPauseKey {
    pub fn new() -> Self {
        Self(false)
    }
    pub fn type_() -> StructTag {
        StructTag {
            address: IOTA_FRAMEWORK_PACKAGE_ID.into(),
            module: DENY_LIST_MODULE.to_owned(),
            name: ident_str!("GlobalPauseKey").to_owned(),
            type_params: vec![],
        }
    }
}

impl MoveTypeTagTrait for GlobalPauseKey {
    fn get_type_tag() -> TypeTag {
        TypeTag::Struct(Box::new(Self::type_()))
    }
}

pub fn check_coin_deny_list_v1_during_signing(
    address: IotaAddress,
    input_objects: &CheckedInputObjects,
    receiving_objects: &ReceivingObjects,
    object_store: &dyn ObjectStore,
) -> UserInputResult {
    let coin_types = input_object_coin_types_for_denylist_check(input_objects, receiving_objects);
    for coin_type in coin_types {
        let Some(deny_list) = get_per_type_coin_deny_list_v1(&coin_type, object_store) else {
            continue;
        };
        if check_global_pause(&deny_list, object_store, None) {
            return Err(UserInputError::CoinTypeGlobalPause { coin_type });
        }
        if check_address_denied_by_config(&deny_list, address, object_store, None) {
            return Err(UserInputError::AddressDeniedForCoin { address, coin_type });
        }
    }
    Ok(())
}

/// Returns 1) whether the coin deny list check passed,
///         2) the deny lists checked
///         2) the number of regulated coin owners checked.
pub fn check_coin_deny_list_v1_during_execution(
    written_objects: &BTreeMap<ObjectID, Object>,
    cur_epoch: EpochId,
    object_store: &dyn ObjectStore,
) -> DenyListResult {
    let mut new_coin_owners = BTreeMap::new();
    for obj in written_objects.values() {
        if obj.is_gas_coin() {
            continue;
        }
        let Some(coin_type) = obj.coin_type_maybe() else {
            continue;
        };
        let Ok(owner) = obj.owner.get_address_owner_address() else {
            continue;
        };
        new_coin_owners
            .entry(coin_type.to_canonical_string(false))
            .or_insert_with(BTreeSet::new)
            .insert(owner);
    }
    let num_non_gas_coin_owners = new_coin_owners.values().map(|v| v.len() as u64).sum();
    let new_regulated_coin_owners = new_coin_owners
        .into_iter()
        .filter_map(|(coin_type, owners)| {
            let deny_list_config = get_per_type_coin_deny_list_v1(&coin_type, object_store)?;
            Some((coin_type, (deny_list_config, owners)))
        })
        .collect::<BTreeMap<_, _>>();
    let result =
        check_new_regulated_coin_owners(new_regulated_coin_owners, cur_epoch, object_store);
    // `num_non_gas_coin_owners` is used to charge for gas. As such we must be
    // extremely careful to not use a number that is not consistent across all
    // validators. For example, relying on the number of coins with a deny list
    // is _not_ consistent since the deny list is created on the first addition
    // to the deny list. But the total number of coins/owners denied would
    // be consistent since we rely on the results from the last epoch (i.e. relying
    // on the Config's internal invariants)
    DenyListResult {
        result,
        num_non_gas_coin_owners,
    }
}

fn check_new_regulated_coin_owners(
    new_regulated_coin_owners: BTreeMap<String, (Config, BTreeSet<IotaAddress>)>,
    cur_epoch: EpochId,
    object_store: &dyn ObjectStore,
) -> Result<(), ExecutionError> {
    for (coin_type, (deny_list, owners)) in new_regulated_coin_owners {
        if check_global_pause(&deny_list, object_store, Some(cur_epoch)) {
            return Err(ExecutionError::new(
                ExecutionErrorKind::CoinTypeGlobalPause { coin_type },
                None,
            ));
        }
        for owner in owners {
            if check_address_denied_by_config(&deny_list, owner, object_store, Some(cur_epoch)) {
                return Err(ExecutionError::new(
                    ExecutionErrorKind::AddressDeniedForCoin {
                        address: owner,
                        coin_type,
                    },
                    None,
                ));
            }
        }
    }
    Ok(())
}

pub fn get_per_type_coin_deny_list_v1(
    coin_type: &String,
    object_store: &dyn ObjectStore,
) -> Option<Config> {
    let config_key = DOFWrapper {
        name: ConfigKey {
            per_type_index: DENY_LIST_COIN_TYPE_INDEX,
            per_type_key: coin_type.as_bytes().to_vec(),
        },
    };
    // TODO: Consider caching the config object UID to avoid repeat deserialization.
    let config: Config =
        get_dynamic_field_from_store(object_store, IOTA_DENY_LIST_OBJECT_ID, &config_key).ok()?;
    Some(config)
}

pub fn check_address_denied_by_config(
    deny_config: &Config,
    address: IotaAddress,
    object_store: &dyn ObjectStore,
    cur_epoch: Option<EpochId>,
) -> bool {
    let address_key = AddressKey(address);
    read_config_setting(object_store, deny_config, address_key, cur_epoch).unwrap_or(false)
}

pub fn check_global_pause(
    deny_config: &Config,
    object_store: &dyn ObjectStore,
    cur_epoch: Option<EpochId>,
) -> bool {
    let global_pause_key = GlobalPauseKey::new();
    read_config_setting(object_store, deny_config, global_pause_key, cur_epoch).unwrap_or(false)
}

pub fn get_deny_list_root_object(object_store: &dyn ObjectStore) -> IotaResult<Object> {
    match object_store.get_object(&IOTA_DENY_LIST_OBJECT_ID) {
        Ok(Some(obj)) => Ok(obj),
        Ok(None) => {
            error!("Deny list object not found");
            Err(IotaError::Storage("Deny list object not found".to_string()))
        }
        Err(err) => {
            error!("Failed to get deny list object: {err}");
            Err(IotaError::Storage(format!(
                "Failed to get deny list object: {err}"
            )))
        }
    }
}

pub fn get_deny_list_obj_initial_shared_version(
    object_store: &dyn ObjectStore,
) -> IotaResult<SequenceNumber> {
    get_deny_list_root_object(object_store).map(|obj| match obj.owner {
        Owner::Shared {
            initial_shared_version,
        } => initial_shared_version,
        _ => unreachable!("Deny list object must be shared"),
    })
}

/// Fetches the setting from a particular config.
/// Reads the value of the setting, giving `newer_value` if the current epoch is
/// greater than `newer_value_epoch`, and `older_value_opt` otherwise.
/// If `cur_epoch` is `None`, the `newer_value` is always returned.
fn read_config_setting<K, V>(
    object_store: &dyn ObjectStore,
    config: &Config,
    setting_name: K,
    cur_epoch: Option<EpochId>,
) -> Option<V>
where
    K: MoveTypeTagTrait + Serialize + DeserializeOwned + fmt::Debug,
    V: Clone + Serialize + DeserializeOwned + fmt::Debug,
{
    let setting: Setting<V> = {
        match get_dynamic_field_from_store(object_store, *config.id.object_id(), &setting_name) {
            Ok(setting) => setting,
            Err(_) => return None,
        }
    };
    setting.read_value(cur_epoch).cloned()
}

/// Returns all unique coin types in canonical string form from the input
/// objects and receiving objects. It filters out IOTA coins since it's known
/// that it's not a regulated coin.
fn input_object_coin_types_for_denylist_check(
    input_objects: &CheckedInputObjects,
    receiving_objects: &ReceivingObjects,
) -> BTreeSet<String> {
    let all_objects = input_objects
        .inner()
        .iter_objects()
        .chain(receiving_objects.iter_objects());
    all_objects
        .filter_map(|obj| {
            if obj.is_gas_coin() {
                None
            } else {
                obj.coin_type_maybe()
                    .map(|type_tag| type_tag.to_canonical_string(false))
            }
        })
        .collect()
}
