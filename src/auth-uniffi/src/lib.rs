// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

uniffi::setup_scaffolding!();

use google_cloud_auth::credentials::Builder as AdcBuilder;
use google_cloud_auth::credentials::CacheableResource;
use google_cloud_auth::credentials::Credentials as RustCredentials;
use google_cloud_auth::credentials::anonymous::Builder as Anonymous;
use http::HeaderName;
use http::HeaderValue;

use std::sync::LazyLock;
use tokio::runtime::Runtime;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime")
});

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AuthError {
    #[error("Cannot create authentication headers: {0}")]
    CreateHeaders(String),
    #[error("Cannot convert authentication headers to strings: {0}")]
    ConvertHeaders(String),
    #[error("Cannot initialize credentials: {0}")]
    Initialize(String),
}

#[derive(uniffi::Record)]
pub struct HeaderField {
    pub key: String,
    pub value: String,
}

impl HeaderField {
    fn from_header(key: &HeaderName, value: &HeaderValue) -> Result<Self, AuthError> {
        let value = value
            .to_str()
            .map_err(|e| AuthError::ConvertHeaders(format!("{e:?}")))?
            .to_string();
        let key = key.as_str().to_string();
        Ok(Self { key, value })
    }
}

#[derive(uniffi::Object)]
pub struct Credentials(RustCredentials);

#[uniffi::export]
impl Credentials {
    #[uniffi::constructor]
    pub fn new() -> Result<Self, AuthError> {
        // google-cloud-auth instantiates a token cache during build() that requires
        // a tokio reactor context, even though build() itself is not an async fn.
        let adc = RUNTIME
            .block_on(async { AdcBuilder::default().build() })
            .map_err(|e| AuthError::Initialize(format!("{e:?}")))?;
        Ok(Self(adc))
    }

    #[uniffi::constructor]
    pub fn anonymous() -> Self {
        Self(Anonymous::default().build())
    }

    pub async fn headers(&self) -> Result<Vec<HeaderField>, AuthError> {
        let creds = self.0.clone();
        RUNTIME
            .spawn(async move {
                let headers = creds
                    .headers(http::Extensions::new())
                    .await
                    .map_err(|e| AuthError::CreateHeaders(format!("{e:?}")))?;
                match headers {
                    CacheableResource::NotModified => {
                        unreachable!("caching is not implemented yet")
                    }
                    CacheableResource::New { data, .. } => data
                        .iter()
                        .map(|(k, v)| HeaderField::from_header(k, v))
                        .collect::<Result<Vec<_>, AuthError>>(),
                }
            })
            .await
            .unwrap_or_else(|e| {
                Err(AuthError::CreateHeaders(format!(
                    "Tokio spawn failed: {e:?}"
                )))
            })
    }
}
