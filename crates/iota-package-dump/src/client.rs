// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, anyhow};
use cynic::{Operation, QueryBuilder, http::ReqwestExt};
use reqwest::IntoUrl;
use serde::{Serialize, de::DeserializeOwned};

pub(crate) struct Client {
    inner: reqwest::Client,
    url: reqwest::Url,
}

impl Client {
    /// Create a new GraphQL client, talking to an IOTA GraphQL service at
    /// `url`.
    pub(crate) fn new(url: impl IntoUrl) -> Result<Self> {
        Ok(Self {
            inner: reqwest::Client::builder()
                .user_agent(concat!("iota-package-dump/", env!("CARGO_PKG_VERSION")))
                .build()
                .context("Failed to create GraphQL client")?,
            url: url.into_url().context("Invalid RPC URL")?,
        })
    }

    pub(crate) async fn query<Q, V>(&self, query: Operation<Q, V>) -> Result<Q>
    where
        V: Serialize,
        Q: DeserializeOwned + QueryBuilder<V> + 'static,
    {
        self.inner
            .post(self.url.clone())
            .run_graphql(query)
            .await
            .context("Failed to send GraphQL query")?
            .data
            .ok_or_else(|| anyhow!("Empty response to query"))
    }
}
