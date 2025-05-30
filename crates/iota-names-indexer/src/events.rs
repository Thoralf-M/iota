// Copyright (c) 2025 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use iota_names::{config::IotaNamesConfig, domain::Domain, registry::NameRecord};
use iota_types::{base_types::IotaAddress, event::Event};
use serde::{Deserialize, Serialize};
use tracing::debug;

pub(crate) enum IotaNamesEvent {
    IotaNamesRegistry(IotaNamesRegistryEvent),
    IotaNamesReverseRegistry(IotaNamesReverseRegistryEvent),
    AuctionStarted(AuctionStartedEvent),
    Bid(BidEvent),
    AuctionExtended(AuctionExtendedEvent),
    AuctionFinalized(AuctionFinalizedEvent),
}

impl IotaNamesEvent {
    pub(crate) fn try_from_event(event: &Event, config: &IotaNamesConfig) -> anyhow::Result<Self> {
        // anyhow::ensure!(
        //     event.package_id == config.package_address.into(),
        //     "Invalid event package: {}",
        //     event.package_id
        // );
        debug!("Processing event: {event:?}");

        Ok(match event.type_.name.as_str() {
            "IotaNamesRegistryEvent" => Self::IotaNamesRegistry(bcs::from_bytes(&event.contents)?),
            "IotaNamesReverseRegistryEvent" => {
                Self::IotaNamesReverseRegistry(bcs::from_bytes(&event.contents)?)
            }
            "AuctionStartedEvent" => Self::AuctionStarted(bcs::from_bytes(&event.contents)?),
            "BidEvent" => Self::Bid(bcs::from_bytes(&event.contents)?),
            "AuctionExtendedEvent" => Self::AuctionExtended(bcs::from_bytes(&event.contents)?),
            "AuctionFinalizedEvent" => Self::AuctionFinalized(bcs::from_bytes(&event.contents)?),
            _ => anyhow::bail!("Invalid event type: {}", event.type_.name),
        })
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct IotaNamesRegistryEvent {
    pub domain: String,
    name_record: NameRecord,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct IotaNamesReverseRegistryEvent {
    default_address: IotaAddress,
    domain: Domain,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AuctionStartedEvent {
    domain: Domain,
    start_timestamp_ms: u64,
    end_timestamp_ms: u64,
    starting_bid: u64,
    bidder: IotaAddress,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct BidEvent {
    domain: Domain,
    bid: u64,
    bidder: IotaAddress,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AuctionExtendedEvent {
    domain: Domain,
    end_timestamp_ms: u64,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AuctionFinalizedEvent {
    domain: Domain,
    start_timestamp_ms: u64,
    end_timestamp_ms: u64,
    winning_bid: u64,
    winner: IotaAddress,
}
