// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::{fmt, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use object_store::{GetResult, path::Path};
use percent_encoding::{NON_ALPHANUMERIC, percent_encode, utf8_percent_encode};
use reqwest::{Client, ClientBuilder};

use crate::object_store::{
    ObjectStoreGetExt,
    http::{DEFAULT_USER_AGENT, get},
};

#[derive(Debug)]
struct GoogleCloudStorageClient {
    client: Client,
    bucket_name_encoded: String,
}

impl GoogleCloudStorageClient {
    pub fn new(bucket: &str) -> Result<Self> {
        let mut builder = ClientBuilder::new().pool_idle_timeout(None);
        builder = builder.user_agent(DEFAULT_USER_AGENT);
        let client = builder.https_only(false).build()?;
        let bucket_name_encoded = percent_encode(bucket.as_bytes(), NON_ALPHANUMERIC).to_string();

        Ok(Self {
            client,
            bucket_name_encoded,
        })
    }

    async fn get(&self, path: &Path) -> Result<GetResult> {
        let url = self.object_url(path);
        get(&url, "gcs", path, &self.client).await
    }

    fn object_url(&self, path: &Path) -> String {
        let encoded = utf8_percent_encode(path.as_ref(), NON_ALPHANUMERIC);
        format!(
            "https://storage.googleapis.com/{}/{}",
            self.bucket_name_encoded, encoded
        )
    }
}

/// Interface for [Google Cloud Storage](https://cloud.google.com/storage/).
#[derive(Debug)]
pub struct GoogleCloudStorage {
    client: Arc<GoogleCloudStorageClient>,
}

impl GoogleCloudStorage {
    pub fn new(bucket: &str) -> Result<Self> {
        let gcs_client = GoogleCloudStorageClient::new(bucket)?;
        Ok(GoogleCloudStorage {
            client: Arc::new(gcs_client),
        })
    }
}

impl fmt::Display for GoogleCloudStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gcs:{}", self.client.bucket_name_encoded)
    }
}

#[async_trait]
impl ObjectStoreGetExt for GoogleCloudStorage {
    async fn get_bytes(&self, location: &Path) -> Result<Bytes> {
        let result = self.client.get(location).await?;
        let bytes = result.bytes().await?;
        Ok(bytes)
    }
}
