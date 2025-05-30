// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::fmt;

use iota_json_rpc_types::IotaRawMoveObject;
use iota_package_management::PublishedAtError;
use iota_sdk::error::Error as SdkError;
use iota_types::{base_types::ObjectID, error::IotaObjectResponseError};
use move_core_types::account_address::AccountAddress;
use move_symbol_pool::Symbol;

#[derive(Debug, thiserror::Error)]
pub struct AggregateError(pub(crate) Vec<Error>);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Cannot check local module for {package}: {message}")]
    CannotCheckLocalModules { package: Symbol, message: String },

    #[error("Could not read a dependency's on-chain object: {0:?}")]
    DependencyObjectReadFailure(SdkError),

    #[error("On-chain package {0} is empty")]
    EmptyOnChainPackage(AccountAddress),

    #[error("Invalid module {name} with error: {message}")]
    InvalidModuleFailure { name: String, message: String },

    #[error("Local version of dependency {address}::{module} was not found.")]
    LocalDependencyNotFound {
        address: AccountAddress,
        module: Symbol,
    },

    #[error("Source package depends on {0} which is not in the linkage table.")]
    MissingDependencyInLinkageTable(AccountAddress),

    #[error("On-chain package depends on {0} which is not a source dependency.")]
    MissingDependencyInSourcePackage(AccountAddress),

    #[error(
        "Local dependency did not match its on-chain version at {address}::{package}::{module}"
    )]
    ModuleBytecodeMismatch {
        address: AccountAddress,
        package: Symbol,
        module: Symbol,
    },

    #[error("Dependency ID contains an IOTA object, not a Move package: {}", .0.0)]
    ObjectFoundWhenPackageExpected(Box<(ObjectID, IotaRawMoveObject)>),

    #[error("Could not deserialize on-chain dependency {address}::{module}.")]
    OnChainDependencyDeserializationError {
        address: AccountAddress,
        module: Symbol,
    },

    #[error("On-chain version of dependency {package}::{module} was not found.")]
    OnChainDependencyNotFound { package: Symbol, module: Symbol },

    #[error("{0}. Please supply an explicit on-chain address for the package")]
    PublishedAt(#[from] PublishedAtError),

    #[error("Dependency object does not exist or was deleted: {0:?}")]
    IotaObjectRefFailure(IotaObjectResponseError),

    #[error("On-chain address cannot be zero")]
    ZeroOnChainAddressSpecifiedFailure,
}

impl fmt::Display for AggregateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Self(errors) = self;
        match &errors[..] {
            [] => unreachable!("Aggregate error with no errors"),
            [error] => write!(f, "{}", error)?,
            errors => {
                writeln!(f, "Multiple source verification errors found:")?;
                for error in errors {
                    write!(f, "\n- {}", error)?;
                }
                return Ok(());
            }
        };
        Ok(())
    }
}

impl From<Error> for AggregateError {
    fn from(error: Error) -> Self {
        Self(vec![error])
    }
}
