// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use diesel::prelude::*;
use iota_json_rpc::coin_api::parse_to_struct_tag;
use iota_json_rpc_types::{Balance, Coin as IotaCoin};
use iota_package_resolver::{PackageStore, Resolver};
use iota_types::{
    base_types::{ObjectID, ObjectRef, SequenceNumber},
    digests::ObjectDigest,
    dynamic_field::{DynamicFieldType, Field},
    object::{Object, ObjectRead, PastObjectRead},
};
use move_core_types::annotated_value::MoveTypeLayout;
use serde::de::DeserializeOwned;

use crate::{
    errors::IndexerError,
    schema::{objects, objects_history, objects_snapshot},
    types::{IndexedDeletedObject, IndexedObject, ObjectStatus, owner_to_owner_info},
};

#[derive(Queryable)]
pub struct ObjectRefColumn {
    pub object_id: Vec<u8>,
    pub object_version: i64,
    pub object_digest: Vec<u8>,
}

// NOTE: please add updating statement like below in pg_indexer_store.rs,
// if new columns are added here:
// objects::epoch.eq(excluded(objects::epoch))
#[derive(Queryable, Insertable, Debug, Identifiable, Clone, QueryableByName)]
#[diesel(table_name = objects, primary_key(object_id))]
pub struct StoredObject {
    pub object_id: Vec<u8>,
    pub object_version: i64,
    pub object_digest: Vec<u8>,
    pub owner_type: i16,
    pub owner_id: Option<Vec<u8>>,
    /// The full type of this object, including package id, module, name and
    /// type parameters. This and following three fields will be None if the
    /// object is a Package
    pub object_type: Option<String>,
    pub object_type_package: Option<Vec<u8>>,
    pub object_type_module: Option<String>,
    /// Name of the object type, e.g., "Coin", without type parameters.
    pub object_type_name: Option<String>,
    pub serialized_object: Vec<u8>,
    pub coin_type: Option<String>,
    // TODO deal with overflow
    pub coin_balance: Option<i64>,
    pub df_kind: Option<i16>,
}

#[derive(Queryable, Insertable, Selectable, Debug, Identifiable, Clone, QueryableByName)]
#[diesel(table_name = objects_snapshot, primary_key(object_id))]
pub struct StoredObjectSnapshot {
    pub object_id: Vec<u8>,
    pub object_version: i64,
    pub object_status: i16,
    pub object_digest: Option<Vec<u8>>,
    pub checkpoint_sequence_number: i64,
    pub owner_type: Option<i16>,
    pub owner_id: Option<Vec<u8>>,
    pub object_type: Option<String>,
    pub object_type_package: Option<Vec<u8>>,
    pub object_type_module: Option<String>,
    pub object_type_name: Option<String>,
    pub serialized_object: Option<Vec<u8>>,
    pub coin_type: Option<String>,
    pub coin_balance: Option<i64>,
    pub df_kind: Option<i16>,
}

impl From<IndexedObject> for StoredObjectSnapshot {
    fn from(o: IndexedObject) -> Self {
        let IndexedObject {
            checkpoint_sequence_number,
            object,
            df_kind,
        } = o;
        let (owner_type, owner_id) = owner_to_owner_info(&object.owner);
        let coin_type = object
            .coin_type_maybe()
            .map(|t| t.to_canonical_string(/* with_prefix */ true));
        let coin_balance = if coin_type.is_some() {
            Some(object.get_coin_value_unsafe())
        } else {
            None
        };

        Self {
            object_id: object.id().to_vec(),
            object_version: object.version().value() as i64,
            object_status: ObjectStatus::Active as i16,
            object_digest: Some(object.digest().into_inner().to_vec()),
            checkpoint_sequence_number: checkpoint_sequence_number as i64,
            owner_type: Some(owner_type as i16),
            owner_id: owner_id.map(|id| id.to_vec()),
            object_type: object
                .type_()
                .map(|t| t.to_canonical_string(/* with_prefix */ true)),
            object_type_package: object.type_().map(|t| t.address().to_vec()),
            object_type_module: object.type_().map(|t| t.module().to_string()),
            object_type_name: object.type_().map(|t| t.name().to_string()),
            serialized_object: Some(bcs::to_bytes(&object).unwrap()),
            coin_type,
            coin_balance: coin_balance.map(|b| b as i64),
            df_kind: df_kind.map(|k| match k {
                DynamicFieldType::DynamicField => 0,
                DynamicFieldType::DynamicObject => 1,
            }),
        }
    }
}

impl From<IndexedDeletedObject> for StoredObjectSnapshot {
    fn from(o: IndexedDeletedObject) -> Self {
        Self {
            object_id: o.object_id.to_vec(),
            object_version: o.object_version as i64,
            object_status: ObjectStatus::WrappedOrDeleted as i16,
            object_digest: None,
            checkpoint_sequence_number: o.checkpoint_sequence_number as i64,
            owner_type: None,
            owner_id: None,
            object_type: None,
            object_type_package: None,
            object_type_module: None,
            object_type_name: None,
            serialized_object: None,
            coin_type: None,
            coin_balance: None,
            df_kind: None,
        }
    }
}

#[derive(Queryable, Insertable, Selectable, Debug, Identifiable, Clone, QueryableByName)]
#[diesel(table_name = objects_history, primary_key(object_id, object_version, checkpoint_sequence_number))]
pub struct StoredHistoryObject {
    pub object_id: Vec<u8>,
    pub object_version: i64,
    pub object_status: i16,
    pub object_digest: Option<Vec<u8>>,
    pub checkpoint_sequence_number: i64,
    pub owner_type: Option<i16>,
    pub owner_id: Option<Vec<u8>>,
    pub object_type: Option<String>,
    pub object_type_package: Option<Vec<u8>>,
    pub object_type_module: Option<String>,
    pub object_type_name: Option<String>,
    pub serialized_object: Option<Vec<u8>>,
    pub coin_type: Option<String>,
    pub coin_balance: Option<i64>,
    pub df_kind: Option<i16>,
}

impl StoredHistoryObject {
    pub async fn try_into_past_object_read(
        self,
        package_resolver: Arc<Resolver<impl PackageStore>>,
    ) -> Result<PastObjectRead, IndexerError> {
        let object_status = ObjectStatus::try_from(self.object_status).map_err(|_| {
            IndexerError::PersistentStorageDataCorruption(format!(
                "Object {} has an invalid object status: {}",
                ObjectID::from_bytes(self.object_id.clone()).unwrap(),
                self.object_status
            ))
        })?;

        if let ObjectStatus::WrappedOrDeleted = object_status {
            let object_ref = (
                ObjectID::from_bytes(self.object_id.clone())?,
                SequenceNumber::from_u64(self.object_version as u64),
                ObjectDigest::OBJECT_DIGEST_DELETED,
            );
            return Ok(PastObjectRead::ObjectDeleted(object_ref));
        }

        let object: Object = self.try_into()?;
        let object_ref = object.compute_object_reference();

        let Some(move_object) = object.data.try_as_move().cloned() else {
            return Ok(PastObjectRead::VersionFound(object_ref, object, None));
        };

        let move_type_layout = package_resolver
            .type_layout(move_object.type_().clone().into())
            .await
            .map_err(|e| {
                IndexerError::ResolveMoveStruct(format!(
                    "failed to convert into object read for obj {}:{}, type: {}. error: {e}",
                    object.id(),
                    object.version(),
                    move_object.type_(),
                ))
            })?;

        let move_struct_layout = match move_type_layout {
            MoveTypeLayout::Struct(s) => Ok(s),
            _ => Err(IndexerError::ResolveMoveStruct(
                "MoveTypeLayout is not a Struct".to_string(),
            )),
        }?;

        Ok(PastObjectRead::VersionFound(
            object_ref,
            object,
            Some(*move_struct_layout),
        ))
    }
}

impl TryFrom<StoredHistoryObject> for Object {
    type Error = IndexerError;

    fn try_from(o: StoredHistoryObject) -> Result<Self, Self::Error> {
        let serialized_object = o.serialized_object.ok_or_else(|| {
            IndexerError::Serde(format!(
                "Failed to deserialize object: {:?}, error: object is None",
                o.object_id
            ))
        })?;

        bcs::from_bytes(&serialized_object).map_err(|e| {
            IndexerError::Serde(format!(
                "Failed to deserialize object: {:?}, error: {e}",
                o.object_id
            ))
        })
    }
}

impl From<IndexedObject> for StoredHistoryObject {
    fn from(o: IndexedObject) -> Self {
        let IndexedObject {
            checkpoint_sequence_number,
            object,
            df_kind,
        } = o;
        let (owner_type, owner_id) = owner_to_owner_info(&object.owner);
        let coin_type = object
            .coin_type_maybe()
            .map(|t| t.to_canonical_string(/* with_prefix */ true));
        let coin_balance = if coin_type.is_some() {
            Some(object.get_coin_value_unsafe())
        } else {
            None
        };

        Self {
            object_id: object.id().to_vec(),
            object_version: object.version().value() as i64,
            object_status: ObjectStatus::Active as i16,
            object_digest: Some(object.digest().into_inner().to_vec()),
            checkpoint_sequence_number: checkpoint_sequence_number as i64,
            owner_type: Some(owner_type as i16),
            owner_id: owner_id.map(|id| id.to_vec()),
            object_type: object
                .type_()
                .map(|t| t.to_canonical_string(/* with_prefix */ true)),
            object_type_package: object.type_().map(|t| t.address().to_vec()),
            object_type_module: object.type_().map(|t| t.module().to_string()),
            object_type_name: object.type_().map(|t| t.name().to_string()),
            serialized_object: Some(bcs::to_bytes(&object).unwrap()),
            coin_type,
            coin_balance: coin_balance.map(|b| b as i64),
            df_kind: df_kind.map(|k| match k {
                DynamicFieldType::DynamicField => 0,
                DynamicFieldType::DynamicObject => 1,
            }),
        }
    }
}

impl From<IndexedDeletedObject> for StoredHistoryObject {
    fn from(o: IndexedDeletedObject) -> Self {
        Self {
            object_id: o.object_id.to_vec(),
            object_version: o.object_version as i64,
            object_status: ObjectStatus::WrappedOrDeleted as i16,
            object_digest: None,
            checkpoint_sequence_number: o.checkpoint_sequence_number as i64,
            owner_type: None,
            owner_id: None,
            object_type: None,
            object_type_package: None,
            object_type_module: None,
            object_type_name: None,
            serialized_object: None,
            coin_type: None,
            coin_balance: None,
            df_kind: None,
        }
    }
}

#[derive(Queryable, Insertable, Debug, Identifiable, Clone, QueryableByName)]
#[diesel(table_name = objects, primary_key(object_id))]
pub struct StoredDeletedObject {
    pub object_id: Vec<u8>,
    pub object_version: i64,
}

impl From<IndexedDeletedObject> for StoredDeletedObject {
    fn from(o: IndexedDeletedObject) -> Self {
        Self {
            object_id: o.object_id.to_vec(),
            object_version: o.object_version as i64,
        }
    }
}

#[derive(Queryable, Insertable, Debug, Identifiable, Clone, QueryableByName)]
#[diesel(table_name = objects_history, primary_key(object_id, object_version, checkpoint_sequence_number))]
pub(crate) struct StoredDeletedHistoryObject {
    pub object_id: Vec<u8>,
    pub object_version: i64,
    pub object_status: i16,
    pub checkpoint_sequence_number: i64,
}

impl From<IndexedObject> for StoredObject {
    fn from(o: IndexedObject) -> Self {
        let IndexedObject {
            checkpoint_sequence_number: _,
            object,
            df_kind,
        } = o;
        let (owner_type, owner_id) = owner_to_owner_info(&object.owner);
        let coin_type = object
            .coin_type_maybe()
            .map(|t| t.to_canonical_string(/* with_prefix */ true));
        let coin_balance = if coin_type.is_some() {
            Some(object.get_coin_value_unsafe())
        } else {
            None
        };
        Self {
            object_id: object.id().to_vec(),
            object_version: object.version().value() as i64,
            object_digest: object.digest().into_inner().to_vec(),
            owner_type: owner_type as i16,
            owner_id: owner_id.map(|id| id.to_vec()),
            object_type: object
                .type_()
                .map(|t| t.to_canonical_string(/* with_prefix */ true)),
            object_type_package: object.type_().map(|t| t.address().to_vec()),
            object_type_module: object.type_().map(|t| t.module().to_string()),
            object_type_name: object.type_().map(|t| t.name().to_string()),
            serialized_object: bcs::to_bytes(&object).unwrap(),
            coin_type,
            coin_balance: coin_balance.map(|b| b as i64),
            df_kind: df_kind.map(|k| match k {
                DynamicFieldType::DynamicField => 0,
                DynamicFieldType::DynamicObject => 1,
            }),
        }
    }
}

impl TryFrom<StoredObject> for Object {
    type Error = IndexerError;

    fn try_from(o: StoredObject) -> Result<Self, Self::Error> {
        bcs::from_bytes(&o.serialized_object).map_err(|e| {
            IndexerError::Serde(format!(
                "Failed to deserialize object: {:?}, error: {}",
                o.object_id, e
            ))
        })
    }
}

impl StoredObject {
    pub async fn try_into_object_read(
        self,
        package_resolver: Arc<Resolver<impl PackageStore>>,
    ) -> Result<ObjectRead, IndexerError> {
        let oref = self.get_object_ref()?;
        let object: iota_types::object::Object = self.try_into()?;

        let Some(move_object) = object.data.try_as_move().cloned() else {
            return Ok(ObjectRead::Exists(oref, object, None));
        };

        let move_type_layout = package_resolver
            .type_layout(move_object.type_().clone().into())
            .await
            .map_err(|e| {
                IndexerError::ResolveMoveStruct(format!(
                    "Failed to convert into object read for obj {}:{}, type: {}. Error: {e}",
                    object.id(),
                    object.version(),
                    move_object.type_(),
                ))
            })?;
        let move_struct_layout = match move_type_layout {
            MoveTypeLayout::Struct(s) => Ok(s),
            _ => Err(IndexerError::ResolveMoveStruct(
                "MoveTypeLayout is not a Struct".to_string(),
            )),
        }?;

        Ok(ObjectRead::Exists(oref, object, Some(*move_struct_layout)))
    }

    pub fn get_object_ref(&self) -> Result<ObjectRef, IndexerError> {
        let object_id = ObjectID::from_bytes(self.object_id.clone()).map_err(|_| {
            IndexerError::Serde(format!("Can't convert {:?} to object_id", self.object_id))
        })?;
        let object_digest =
            ObjectDigest::try_from(self.object_digest.as_slice()).map_err(|_| {
                IndexerError::Serde(format!(
                    "Can't convert {:?} to object_digest",
                    self.object_digest
                ))
            })?;
        Ok((
            object_id,
            (self.object_version as u64).into(),
            object_digest,
        ))
    }

    pub fn to_dynamic_field<K, V>(&self) -> Option<Field<K, V>>
    where
        K: DeserializeOwned,
        V: DeserializeOwned,
    {
        let object: Object = bcs::from_bytes(&self.serialized_object).ok()?;

        let object = object.data.try_as_move()?;
        let ty = object.type_();

        if !ty.is_dynamic_field() {
            return None;
        }

        bcs::from_bytes(object.contents()).ok()
    }
}

impl TryFrom<StoredObject> for IotaCoin {
    type Error = IndexerError;

    fn try_from(o: StoredObject) -> Result<Self, Self::Error> {
        let object: Object = o.clone().try_into()?;
        let (coin_object_id, version, digest) = o.get_object_ref()?;
        let coin_type_canonical =
            o.coin_type
                .ok_or(IndexerError::PersistentStorageDataCorruption(format!(
                    "Object {} is supposed to be a coin but has an empty coin_type column",
                    coin_object_id,
                )))?;
        let coin_type = parse_to_struct_tag(coin_type_canonical.as_str())
            .map_err(|_| {
                IndexerError::PersistentStorageDataCorruption(format!(
                    "The type of object {} cannot be parsed as a struct tag",
                    coin_object_id,
                ))
            })?
            .to_string();
        let balance = o
            .coin_balance
            .ok_or(IndexerError::PersistentStorageDataCorruption(format!(
                "Object {} is supposed to be a coin but has an empty coin_balance column",
                coin_object_id,
            )))?;
        Ok(IotaCoin {
            coin_type,
            coin_object_id,
            version,
            digest,
            balance: balance as u64,
            previous_transaction: object.previous_transaction,
        })
    }
}

#[derive(QueryableByName)]
pub struct CoinBalance {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub coin_type: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub coin_num: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub coin_balance: i64,
}

impl TryFrom<CoinBalance> for Balance {
    type Error = IndexerError;

    fn try_from(c: CoinBalance) -> Result<Self, Self::Error> {
        let coin_type = parse_to_struct_tag(c.coin_type.as_str())
            .map_err(|_| {
                IndexerError::PersistentStorageDataCorruption(
                    "The type of coin balance cannot be parsed as a struct tag".to_string(),
                )
            })?
            .to_string();
        Ok(Self {
            coin_type,
            coin_object_count: c.coin_num as usize,
            // TODO: deal with overflow
            total_balance: c.coin_balance as u128,
        })
    }
}

#[cfg(test)]
mod tests {
    use iota_types::{
        Identifier, TypeTag,
        coin::Coin,
        digests::TransactionDigest,
        gas_coin::{GAS, GasCoin},
        object::{Data, MoveObject, ObjectInner, Owner},
    };
    use move_core_types::{account_address::AccountAddress, language_storage::StructTag};

    use super::*;

    #[test]
    fn test_canonical_string_of_object_type_for_coin() {
        let test_obj = Object::new_gas_for_testing();
        let indexed_obj = IndexedObject::from_object(1, test_obj, None);

        let stored_obj = StoredObject::from(indexed_obj);

        match stored_obj.object_type {
            Some(t) => {
                assert_eq!(
                    t,
                    "0x0000000000000000000000000000000000000000000000000000000000000002::coin::Coin<0x0000000000000000000000000000000000000000000000000000000000000002::iota::IOTA>"
                );
            }
            None => {
                panic!("object_type should not be none");
            }
        }
    }

    #[test]
    fn test_convert_stored_obj_to_iota_coin() {
        let test_obj = Object::new_gas_for_testing();
        let indexed_obj = IndexedObject::from_object(1, test_obj, None);

        let stored_obj = StoredObject::from(indexed_obj);

        let iota_coin = IotaCoin::try_from(stored_obj).unwrap();
        assert_eq!(iota_coin.coin_type, "0x2::iota::IOTA");
    }

    #[test]
    fn test_output_format_coin_balance() {
        let test_obj = Object::new_gas_for_testing();
        let indexed_obj = IndexedObject::from_object(1, test_obj, None);

        let stored_obj = StoredObject::from(indexed_obj);
        let test_balance = CoinBalance {
            coin_type: stored_obj.coin_type.unwrap(),
            coin_num: 1,
            coin_balance: 100,
        };
        let balance = Balance::try_from(test_balance).unwrap();
        assert_eq!(balance.coin_type, "0x2::iota::IOTA");
    }

    #[test]
    fn test_vec_of_coin_iota_conversion() {
        // 0xe7::vec_coin::VecCoin<vector<0x2::coin::Coin<0x2::iota::IOTA>>>
        let vec_coins_type = TypeTag::Vector(Box::new(
            Coin::type_(TypeTag::Struct(Box::new(GAS::type_()))).into(),
        ));
        let object_type = StructTag {
            address: AccountAddress::from_hex_literal("0xe7").unwrap(),
            module: Identifier::new("vec_coin").unwrap(),
            name: Identifier::new("VecCoin").unwrap(),
            type_params: vec![vec_coins_type],
        };

        let id = ObjectID::ZERO;
        let gas = 10;

        let contents = bcs::to_bytes(&vec![GasCoin::new(id, gas)]).unwrap();
        let data = Data::Move(
            {
                MoveObject::new_from_execution_with_limit(
                    object_type.into(),
                    1.into(),
                    contents,
                    256,
                )
            }
            .unwrap(),
        );

        let owner = AccountAddress::from_hex_literal("0x1").unwrap();

        let object = ObjectInner {
            owner: Owner::AddressOwner(owner.into()),
            data,
            previous_transaction: TransactionDigest::genesis_marker(),
            storage_rebate: 0,
        }
        .into();

        let indexed_obj = IndexedObject::from_object(1, object, None);

        let stored_obj = StoredObject::from(indexed_obj);

        match stored_obj.object_type {
            Some(t) => {
                assert_eq!(
                    t,
                    "0x00000000000000000000000000000000000000000000000000000000000000e7::vec_coin::VecCoin<vector<0x0000000000000000000000000000000000000000000000000000000000000002::coin::Coin<0x0000000000000000000000000000000000000000000000000000000000000002::iota::IOTA>>>"
                );
            }
            None => {
                panic!("object_type should not be none");
            }
        }
    }
}
