// Copyright 2024 Google LLC
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

use auth::credentials::Credentials;
use http::HeaderMap;
use tokio::sync::watch;
use tokio::time::{Duration, sleep};
use tonic::service::Interceptor;
use tonic::{Request, Status};

const REFRESH_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes
const ERROR_RETRY_DELAY: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct GcpInterceptor {
    rx: watch::Receiver<Option<HeaderMap>>,
}

impl GcpInterceptor {
    pub fn new(credentials: Credentials) -> Self {
        let (tx, rx) = watch::channel(None);
        tokio::spawn(refresh_task(credentials, tx));
        Self { rx }
    }

    #[cfg(test)]
    pub(crate) fn from_rx(rx: watch::Receiver<Option<HeaderMap>>) -> Self {
        Self { rx }
    }
}

impl Interceptor for GcpInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let rx_ref = self.rx.borrow();
        if let Some(headers) = rx_ref.as_ref() {
            for (name, value) in headers.iter() {
                let key = tonic::metadata::MetadataKey::from_bytes(name.as_str().as_bytes())
                    .map_err(|e| Status::internal(format!("invalid header name: {e}")))?;
                let val = tonic::metadata::MetadataValue::try_from(value.as_bytes())
                    .map_err(|e| Status::internal(format!("invalid header value: {e}")))?;
                request.metadata_mut().insert(key, val);
            }
            Ok(request)
        } else {
            Err(Status::unauthenticated("GCP credentials not yet available"))
        }
    }
}

async fn refresh_task(credentials: Credentials, tx: watch::Sender<Option<HeaderMap>>) {
    loop {
        match credentials.headers(http::Extensions::new()).await {
            Ok(auth::credentials::CacheableResource::New { data, .. }) => {
                if tx.send(Some(data)).is_err() {
                    // Receiver dropped, stop task
                    break;
                }
                sleep(REFRESH_INTERVAL).await;
            }
            Ok(auth::credentials::CacheableResource::NotModified) => {
                // Should not happen with empty extensions, but just in case
                sleep(REFRESH_INTERVAL).await;
            }
            Err(e) => {
                tracing::warn!("Failed to refresh GCP credentials: {e:?}");
                sleep(ERROR_RETRY_DELAY).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[tokio::test]
    async fn test_interceptor_injects_headers() {
        let (tx, rx) = watch::channel(None);
        let mut interceptor = GcpInterceptor { rx };

        // 1. Initial state (no headers)
        let req = Request::new(());
        let res = interceptor.call(req);
        assert!(matches!(
            res,
            Err(status) if status.code() == tonic::Code::Unauthenticated
        ));

        // 2. Send headers
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Bearer test-token"),
        );
        headers.insert(
            "x-goog-user-project",
            HeaderValue::from_static("test-project"),
        );
        tx.send(Some(headers)).unwrap();

        // 3. Verify injection
        let req = Request::new(());
        let res = interceptor.call(req).unwrap();
        let metadata = res.metadata();
        assert_eq!(metadata.get("authorization").unwrap(), "Bearer test-token");
        assert_eq!(metadata.get("x-goog-user-project").unwrap(), "test-project");
    }
}
