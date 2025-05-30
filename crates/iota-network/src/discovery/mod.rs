// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use anemo::{
    Network, Peer, PeerId, Request, Response,
    types::{PeerEvent, PeerInfo},
};
use futures::StreamExt;
use iota_config::p2p::{AccessType, DiscoveryConfig, P2pConfig, SeedPeer};
use iota_types::multiaddr::Multiaddr;
use serde::{Deserialize, Serialize};
use tap::{Pipe, TapFallible};
use tokio::{
    sync::{broadcast::error::RecvError, oneshot, watch},
    task::{AbortHandle, JoinSet},
};
use tracing::{debug, info, trace};

const TIMEOUT: Duration = Duration::from_secs(1);
const ONE_DAY_MILLISECONDS: u64 = 24 * 60 * 60 * 1_000;

// Includes the generated Discovery code from the OUT_DIR
mod generated {
    include!(concat!(env!("OUT_DIR"), "/iota.Discovery.rs"));
}
mod builder;
mod metrics;
mod server;
#[cfg(test)]
mod tests;

pub use builder::{Builder, Handle, UnstartedDiscovery};
pub use generated::{
    discovery_client::DiscoveryClient,
    discovery_server::{Discovery, DiscoveryServer},
};
pub use server::GetKnownPeersResponse;

use self::metrics::Metrics;

/// The internal discovery state shared between the main event loop and the
/// request handler
struct State {
    our_info: Option<NodeInfo>,
    connected_peers: HashMap<PeerId, ()>,
    known_peers: HashMap<PeerId, NodeInfo>,
}

/// The information necessary to dial another peer.
///
/// `NodeInfo` contains all the information that is shared with other nodes via
/// the discovery service to advertise how a node can be reached.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeInfo {
    pub peer_id: PeerId,
    pub addresses: Vec<Multiaddr>,

    /// Creation time.
    ///
    /// This is used to determine which of two NodeInfo's from the same PeerId
    /// should be retained.
    pub timestamp_ms: u64,

    pub access_type: AccessType,
}

#[derive(Clone, Debug, Default)]
/// Contains a new list of available trusted peers.
pub struct TrustedPeerChangeEvent {
    pub new_committee: Vec<PeerInfo>,
    pub old_committee: Vec<PeerInfo>,
}

struct DiscoveryEventLoop {
    config: P2pConfig,
    discovery_config: Arc<DiscoveryConfig>,
    allowlisted_peers: Arc<HashMap<PeerId, Option<Multiaddr>>>,
    network: Network,
    tasks: JoinSet<()>,
    pending_dials: HashMap<PeerId, AbortHandle>,
    dial_seed_peers_task: Option<AbortHandle>,
    shutdown_handle: oneshot::Receiver<()>,
    state: Arc<RwLock<State>>,
    trusted_peer_change_rx: watch::Receiver<TrustedPeerChangeEvent>,
    metrics: Metrics,
}

impl DiscoveryEventLoop {
    /// Starts the discovery event loop.
    pub async fn start(mut self) {
        info!("Discovery started");

        self.construct_our_info();
        self.configure_preferred_peers();

        let mut interval = tokio::time::interval(self.discovery_config.interval_period());
        let mut peer_events = {
            let (subscriber, _peers) = self.network.subscribe().unwrap();
            subscriber
        };

        loop {
            tokio::select! {
                now = interval.tick() => {
                    let now_unix = now_unix();
                    self.handle_tick(now.into_std(), now_unix);
                }
                peer_event = peer_events.recv() => {
                    self.handle_peer_event(peer_event);
                },
                // This is signaled when new trusted peer (committee member) is added.
                Ok(()) = self.trusted_peer_change_rx.changed() => {
                    let event: TrustedPeerChangeEvent = self.trusted_peer_change_rx.borrow_and_update().clone();
                    self.handle_trusted_peer_change_event(event);
                }
                // Handles the result of a task from tasks.
                Some(task_result) = self.tasks.join_next() => {
                    match task_result {
                        Ok(()) => {},
                        Err(e) => {
                            if e.is_cancelled() {
                                // avoid crashing on ungraceful shutdown.
                            } else if e.is_panic() {
                                // propagate panics.
                                std::panic::resume_unwind(e.into_panic());
                            } else {
                                panic!("task failed: {e}");
                            }
                        },
                    };
                },
                // Once the shutdown notification is resolved we can terminate the event loop.
                _ = &mut self.shutdown_handle => {
                    break;
                }
            }
        }

        info!("Discovery ended");
    }

    /// Constructs [`NodeInfo`] of the node.
    fn construct_our_info(&mut self) {
        if self.state.read().unwrap().our_info.is_some() {
            return;
        }

        let address = self
            .config
            .external_address
            .clone()
            .and_then(|addr| addr.to_anemo_address().ok().map(|_| addr))
            .into_iter()
            .collect();
        let our_info = NodeInfo {
            peer_id: self.network.peer_id(),
            addresses: address,
            timestamp_ms: now_unix(),
            access_type: self.discovery_config.access_type(),
        };

        self.state.write().unwrap().our_info = Some(our_info);
    }

    /// Configures known peers list in [`Network`] using allowlisted peers and
    /// seed peers.
    fn configure_preferred_peers(&mut self) {
        // Iterates over the allowlisted peers and seed peers to check if they have
        // an address that can be converted to anemo address. If they do, they are added
        // to the known peers list.
        for (peer_id, address) in self
            .discovery_config
            .allowlisted_peers
            .iter()
            .map(|ap| (ap.peer_id, ap.address.clone()))
            .chain(self.config.seed_peers.iter().filter_map(|sp| {
                sp.peer_id
                    .map(|peer_id| (peer_id, Some(sp.address.clone())))
            }))
        {
            let anemo_address = if let Some(address) = address {
                let Ok(address) = address.to_anemo_address() else {
                    debug!(p2p_address=?address, "Can't convert p2p address to anemo address");
                    continue;
                };
                Some(address)
            } else {
                None
            };

            // TODO: once we have `PeerAffinity::Allowlisted` we should update allowlisted
            // peers' affinity.
            let peer_info = anemo::types::PeerInfo {
                peer_id,
                affinity: anemo::types::PeerAffinity::High,
                address: anemo_address.into_iter().collect(),
            };
            debug!(?peer_info, "Add configured preferred peer");
            self.network.known_peers().insert(peer_info);
        }
    }

    fn update_our_info_timestamp(&mut self, now_unix: u64) {
        if let Some(our_info) = &mut self.state.write().unwrap().our_info {
            our_info.timestamp_ms = now_unix;
        }
    }

    /// Handles a [`TrustedPeerChangeEvent`] by updating the known peers with
    /// the latest trusted new peers without deleting the allowlisted peers.
    fn handle_trusted_peer_change_event(
        &mut self,
        trusted_peer_change_event: TrustedPeerChangeEvent,
    ) {
        let TrustedPeerChangeEvent {
            new_committee,
            old_committee,
        } = trusted_peer_change_event;

        let new_peer_ids = new_committee
            .iter()
            .map(|peer| peer.peer_id)
            .collect::<HashSet<_>>();

        // Remove peers from old_committee who are not in new_committee and are not in
        // self.allowlisted_peers.
        let to_remove = old_committee
            .iter()
            .map(|peer_info| &peer_info.peer_id)
            .filter(|old_peer_id| {
                !new_peer_ids.contains(old_peer_id)
                    && !self.allowlisted_peers.contains_key(old_peer_id)
            });

        // Add the new_committee to the known peers skipping self peer.
        // This will update the PeerInfo for those who are already in the
        // committee and have updated their PeerInfo.
        let to_insert = new_committee
            .into_iter()
            .filter(|peer_info| !self.network.peer_id().eq(&peer_info.peer_id));

        let (removed, updated_or_inserted) = self
            .network
            .known_peers()
            .batch_update(to_remove, to_insert.clone());

        // Actually removed, may differ from `to_remove`
        let removed: Vec<_> = removed
            .into_iter()
            .filter_map(|removed| removed.map(|info| info.peer_id))
            .collect();
        let mut updated = Vec::new();
        let mut inserted = Vec::new();
        for (replaced_val, to_insert_val) in updated_or_inserted.into_iter().zip(to_insert) {
            if replaced_val.is_some() {
                updated.push(to_insert_val.peer_id);
            } else {
                inserted.push(to_insert_val.peer_id);
            }
        }
        debug!(
            "Trusted peer change event: removed {removed:?}, updated {updated:?}, inserted {inserted:?}",
        );
    }

    /// Handles a [`PeerEvent`].
    ///
    /// * NewPeer: Adds the peer to the connected peers list and queries the
    ///   peer for their known peers.
    /// * LostPeer: Removes the peer from the connected peers list.
    /// * Closed: Panics if the channel is closed.
    fn handle_peer_event(&mut self, peer_event: Result<PeerEvent, RecvError>) {
        match peer_event {
            Ok(PeerEvent::NewPeer(peer_id)) => {
                if let Some(peer) = self.network.peer(peer_id) {
                    // Adds the peer to the connected peers list.
                    self.state
                        .write()
                        .unwrap()
                        .connected_peers
                        .insert(peer_id, ());

                    // Queries the new node for any peers.
                    self.tasks.spawn(query_peer_for_their_known_peers(
                        peer,
                        self.state.clone(),
                        self.metrics.clone(),
                        self.allowlisted_peers.clone(),
                    ));
                }
            }
            Ok(PeerEvent::LostPeer(peer_id, _)) => {
                self.state.write().unwrap().connected_peers.remove(&peer_id);
            }

            Err(RecvError::Closed) => {
                panic!("PeerEvent channel shouldn't be able to be closed");
            }

            Err(RecvError::Lagged(_)) => {
                trace!("State-Sync fell behind processing PeerEvents");
            }
        }
    }

    /// This function performs several tasks:
    ///
    /// 1. Update the timestamp of our own info.
    /// 2. Queries a subset of connected peers for their known peers.
    /// 3. Culls old known peers older than a day.
    /// 4. Cleans out the pending_dials, dial_seed_peers_task if it's done.
    /// 5. Selects a subset of known peers to dial if we're not connected to
    ///    enough peers.
    /// 6. If we have no neighbors and we aren't presently trying to connect to
    ///    anyone we need to try the seed peers.
    fn handle_tick(&mut self, _now: std::time::Instant, now_unix: u64) {
        self.update_our_info_timestamp(now_unix);

        self.tasks
            .spawn(query_connected_peers_for_their_known_peers(
                self.network.clone(),
                self.discovery_config.clone(),
                self.state.clone(),
                self.metrics.clone(),
                self.allowlisted_peers.clone(),
            ));

        // Culls old known peers older than a day.
        self.state
            .write()
            .unwrap()
            .known_peers
            .retain(|_k, v| now_unix.saturating_sub(v.timestamp_ms) < ONE_DAY_MILLISECONDS);

        // Cleans out the pending_dials.
        self.pending_dials.retain(|_k, v| !v.is_finished());
        // Cleans out the dial_seed_peers_task if it's done.
        if let Some(abort_handle) = &self.dial_seed_peers_task {
            if abort_handle.is_finished() {
                self.dial_seed_peers_task = None;
            }
        }

        // Selects a subset of known peers to dial if we're not connected to enough
        // peers.
        let state = self.state.read().unwrap();
        let eligible = state
            .known_peers
            .clone()
            .into_iter()
            .filter(|(peer_id, info)| {
                peer_id != &self.network.peer_id() &&
                !info.addresses.is_empty() // Peer has addresses we can dial
                && !state.connected_peers.contains_key(peer_id) // We're not already connected
                && !self.pending_dials.contains_key(peer_id) // There is no
                // pending dial to
                // this node
            })
            .collect::<Vec<_>>();

        // No need to connect to any more peers if we're already connected to a bunch
        let number_of_connections = state.connected_peers.len();
        let number_to_dial = std::cmp::min(
            eligible.len(),
            self.discovery_config
                .target_concurrent_connections()
                .saturating_sub(number_of_connections),
        );

        // Randomly selects the number_to_dial of peers to connect to.
        for (peer_id, info) in rand::seq::SliceRandom::choose_multiple(
            eligible.as_slice(),
            &mut rand::thread_rng(),
            number_to_dial,
        ) {
            let abort_handle = self.tasks.spawn(try_to_connect_to_peer(
                self.network.clone(),
                info.to_owned(),
            ));
            self.pending_dials.insert(*peer_id, abort_handle);
        }

        // If we aren't connected to anything and we aren't presently trying to connect
        // to anyone we need to try the seed peers
        if self.dial_seed_peers_task.is_none()
            && state.connected_peers.is_empty()
            && self.pending_dials.is_empty()
            && !self.config.seed_peers.is_empty()
        {
            let abort_handle = self.tasks.spawn(try_to_connect_to_seed_peers(
                self.network.clone(),
                self.discovery_config.clone(),
                self.config.seed_peers.clone(),
            ));

            self.dial_seed_peers_task = Some(abort_handle);
        }
    }
}

async fn try_to_connect_to_peer(network: Network, info: NodeInfo) {
    debug!("Connecting to peer {info:?}");
    for multiaddr in &info.addresses {
        if let Ok(address) = multiaddr.to_anemo_address() {
            // Ignore the result and just log the error if there is one
            if network
                .connect_with_peer_id(address, info.peer_id)
                .await
                .tap_err(|e| {
                    debug!(
                        "error dialing {} at address '{}': {e}",
                        info.peer_id.short_display(4),
                        multiaddr
                    )
                })
                .is_ok()
            {
                return;
            }
        }
    }
}

async fn try_to_connect_to_seed_peers(
    network: Network,
    config: Arc<DiscoveryConfig>,
    seed_peers: Vec<SeedPeer>,
) {
    debug!(?seed_peers, "Connecting to seed peers");
    let network = &network;

    futures::stream::iter(seed_peers.into_iter().filter_map(|seed| {
        seed.address
            .to_anemo_address()
            .ok()
            .map(|address| (seed, address))
    }))
    .for_each_concurrent(
        config.target_concurrent_connections(),
        |(seed, address)| async move {
            // Ignores the result and just logs the error if there is one.
            let _ = if let Some(peer_id) = seed.peer_id {
                network.connect_with_peer_id(address, peer_id).await
            } else {
                network.connect(address).await
            }
            .tap_err(|e| debug!("error dialing multiaddr '{}': {e}", seed.address));
        },
    )
    .await;
}

async fn query_peer_for_their_known_peers(
    peer: Peer,
    state: Arc<RwLock<State>>,
    metrics: Metrics,
    allowlisted_peers: Arc<HashMap<PeerId, Option<Multiaddr>>>,
) {
    let mut client = DiscoveryClient::new(peer);

    let request = Request::new(()).with_timeout(TIMEOUT);
    if let Some(found_peers) = client
        .get_known_peers(request)
        .await
        .ok()
        .map(Response::into_inner)
        .map(
            |GetKnownPeersResponse {
                 own_info,
                 mut known_peers,
             }| {
                if !own_info.addresses.is_empty() {
                    known_peers.push(own_info)
                }
                known_peers
            },
        )
    {
        update_known_peers(state, metrics, found_peers, allowlisted_peers);
    }
}

/// Queries a subset of neighbors for their known peers.
async fn query_connected_peers_for_their_known_peers(
    network: Network,
    config: Arc<DiscoveryConfig>,
    state: Arc<RwLock<State>>,
    metrics: Metrics,
    allowlisted_peers: Arc<HashMap<PeerId, Option<Multiaddr>>>,
) {
    use rand::seq::IteratorRandom;

    // Randomly selects a subset of neighbors to query.
    let peers_to_query = network
        .peers()
        .into_iter()
        .flat_map(|id| network.peer(id))
        .choose_multiple(&mut rand::thread_rng(), config.peers_to_query());

    // Queries the selected neighbors for their known peers in parallel.
    let found_peers = peers_to_query
        .into_iter()
        .map(DiscoveryClient::new)
        .map(|mut client| async move {
            let request = Request::new(()).with_timeout(TIMEOUT);
            client
                .get_known_peers(request)
                .await
                .ok()
                .map(Response::into_inner)
                .map(
                    |GetKnownPeersResponse {
                         own_info,
                         mut known_peers,
                     }| {
                        known_peers.push(own_info);
                        known_peers
                    },
                )
        })
        .pipe(futures::stream::iter)
        .buffer_unordered(config.peers_to_query())
        .filter_map(std::future::ready)
        .flat_map(futures::stream::iter)
        .collect::<Vec<_>>()
        .await;

    update_known_peers(state, metrics, found_peers, allowlisted_peers);
}

/// Updates the known peers list with the found peers. The found peer is ignored
/// if it is too old or too far in the future from our clock.
/// If a peer is already known, the NodeInfo is updated, otherwise the peer is
/// inserted.
fn update_known_peers(
    state: Arc<RwLock<State>>,
    metrics: Metrics,
    found_peers: Vec<NodeInfo>,
    allowlisted_peers: Arc<HashMap<PeerId, Option<Multiaddr>>>,
) {
    use std::collections::hash_map::Entry;

    let now_unix = now_unix();
    let our_peer_id = state.read().unwrap().our_info.clone().unwrap().peer_id;
    let known_peers = &mut state.write().unwrap().known_peers;
    for peer in found_peers {
        // Skip peers whose timestamp is too far in the future from our clock
        // or that are too old
        if peer.timestamp_ms > now_unix.saturating_add(30 * 1_000) // 30 seconds
            || now_unix.saturating_sub(peer.timestamp_ms) > ONE_DAY_MILLISECONDS
        {
            continue;
        }

        if peer.peer_id == our_peer_id {
            continue;
        }

        // If Peer is Private, and not in our allowlist, skip it.
        if peer.access_type == AccessType::Private && !allowlisted_peers.contains_key(&peer.peer_id)
        {
            continue;
        }

        match known_peers.entry(peer.peer_id) {
            // Updates the NodeInfo of the peer if it exists.
            Entry::Occupied(mut o) => {
                if peer.timestamp_ms > o.get().timestamp_ms {
                    if o.get().addresses.is_empty() && !peer.addresses.is_empty() {
                        metrics.inc_num_peers_with_external_address();
                    }
                    if !o.get().addresses.is_empty() && peer.addresses.is_empty() {
                        metrics.dec_num_peers_with_external_address();
                    }
                    o.insert(peer);
                }
            }
            // Inserts the peer if it doesn't exist.
            Entry::Vacant(v) => {
                if !peer.addresses.is_empty() {
                    metrics.inc_num_peers_with_external_address();
                }
                v.insert(peer);
            }
        }
    }
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
