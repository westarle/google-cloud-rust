// Copyright 2025 Google LLC
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

use super::client::{StorageInner, apply_customer_supplied_encryption_headers};
use crate::model::Object;
use crate::retry_policy::ContinueOn308;
use crate::storage::checksum::details::ChecksummedSource;
use crate::storage::info::X_GOOG_API_CLIENT_HEADER;
use crate::storage::v1;
use crate::streaming_source::{IterSource, Seek, SizeHint, StreamingSource};
use crate::{Error, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

mod buffered;
mod unbuffered;

/// Represents an upload constructed via `WriteObject<T>`.
///
/// Once the application has fully configured an `WriteObject<T>` it calls
/// `send()` or `send_buffered()` to initiate the upload. At that point the
/// client library creates an instance of this class. Notably, the `payload`
/// becomes `Arc<Mutex<T>>` because it needs to be reused in the retry loop.
pub struct PerformUpload<S> {
    // We need `Arc<Mutex<>>` because this is re-used in retryable uploads.
    payload: Arc<Mutex<ChecksummedSource<S>>>,
    inner: Arc<StorageInner>,
    spec: crate::model::WriteObjectSpec,
    params: Option<crate::model::CommonObjectRequestParams>,
    options: super::request_options::RequestOptions,
}

impl<S> PerformUpload<S> {
    pub(crate) fn new(
        payload: S,
        inner: Arc<StorageInner>,
        spec: crate::model::WriteObjectSpec,
        params: Option<crate::model::CommonObjectRequestParams>,
        options: super::request_options::RequestOptions,
    ) -> Self {
        let checksum = options.checksum.clone();
        Self {
            payload: Arc::new(Mutex::new(ChecksummedSource::new(checksum, payload))),
            inner,
            spec,
            params,
            options,
        }
    }

    fn resource(&self) -> &crate::model::Object {
        self.spec
            .resource
            .as_ref()
            .expect("resource field initialized in `new()`")
    }

    async fn start_resumable_upload_attempt(&self) -> Result<String> {
        if self.inner.use_legacy_transport {
            let builder = self.start_resumable_upload_request().await?;
            let response = builder.send().await.map_err(Error::io)?;
            return self::handle_start_resumable_upload_response(response).await;
        }

        let builder = self.start_resumable_upload_request_v2()?;
        let response = self
            .inner
            .http_client
            .execute_streaming_once(builder, self.options.gax(), None, 0)
            .await?;
        self::handle_start_resumable_upload_response_v2(response).await
    }

    async fn start_resumable_upload_request(&self) -> Result<reqwest::RequestBuilder> {
        let bucket = &self.resource().bucket;
        let bucket_id = bucket.strip_prefix("projects/_/buckets/").ok_or_else(|| {
            Error::binding(format!(
                "malformed bucket name, it must start with `projects/_/buckets/`: {bucket}"
            ))
        })?;
        let object = &self.resource().name;
        let builder = self
            .inner
            .client
            .request(
                reqwest::Method::POST,
                format!("{}/upload/storage/v1/b/{bucket_id}/o", &self.inner.endpoint),
            )
            .query(&[("uploadType", "resumable")])
            .query(&[("name", object)])
            .header("content-type", "application/json")
            .header(
                "x-goog-api-client",
                reqwest::header::HeaderValue::from_static(&X_GOOG_API_CLIENT_HEADER),
            );

        let builder = self.apply_preconditions(builder);
        let builder = apply_customer_supplied_encryption_headers(builder, &self.params);
        let builder = self.inner.apply_auth_headers(builder).await?;
        let builder = builder.json(&v1::insert_body(self.resource()));
        Ok(builder)
    }

    fn start_resumable_upload_request_v2(&self) -> Result<reqwest::RequestBuilder> {
        let bucket = &self.resource().bucket;
        let bucket_id = bucket.strip_prefix("projects/_/buckets/").ok_or_else(|| {
            Error::binding(format!(
                "malformed bucket name, it must start with `projects/_/buckets/`: {bucket}"
            ))
        })?;
        let object = &self.resource().name;
        let builder = self
            .inner
            .http_client
            .builder(
                reqwest::Method::POST,
                format!("upload/storage/v1/b/{bucket_id}/o"),
            )
            .query(&[("uploadType", "resumable")])
            .query(&[("name", object)])
            .header("content-type", "application/json")
            .header(
                "x-goog-api-client",
                reqwest::header::HeaderValue::from_static(&X_GOOG_API_CLIENT_HEADER),
            );

        let builder = self.apply_preconditions(builder);
        let builder = apply_customer_supplied_encryption_headers(builder, &self.params);
        // Auth headers are applied by ReqwestClient
        let builder = builder.json(&v1::insert_body(self.resource()));
        Ok(builder)
    }

    async fn query_resumable_upload_attempt(
        &self,
        upload_url: &str,
    ) -> Result<ResumableUploadStatus> {
        if self.inner.use_legacy_transport {
            let builder = self
                .inner
                .client
                .request(reqwest::Method::PUT, upload_url)
                .header("content-type", "application/octet-stream")
                .header("Content-Range", "bytes */*")
                .header("content-length", 0)
                .header(
                    "x-goog-api-client",
                    reqwest::header::HeaderValue::from_static(&X_GOOG_API_CLIENT_HEADER),
                );
            let builder = self.inner.apply_auth_headers(builder).await?;
            let response = builder.send().await.map_err(Error::io)?;
            return self::query_resumable_upload_handle_response(response).await;
        }

        // For query, we use the upload_url directly.
        // ReqwestClient::builder appends endpoint, but upload_url is absolute?
        // If upload_url is absolute, we should use reqwest::Client directly or handle it.
        // But we want to use ReqwestClient for tracing/auth?
        // Wait, upload_url is usually absolute (returned by Location header).
        // If it's absolute, `ReqwestClient::builder` might double-prepend endpoint if we are not careful.
        // `ReqwestClient::builder` does: `self.inner.request(method, format!("{}{path}", &self.endpoint))`
        // If path is absolute URL, `reqwest` might handle it?
        // No, `format!("{}{path}", ...)` will prepend endpoint.
        // If endpoint is `https://storage.googleapis.com/` and path is `https://storage.googleapis.com/...`, we get double URL.

        // We need to handle this.
        // `ReqwestClient` doesn't seem to expose a way to use absolute URL directly without endpoint prepending,
        // UNLESS we pass empty path and configure builder?
        // But `builder` takes path.

        // Actually, `ReqwestClient` is designed for API calls relative to endpoint.
        // Resumable upload URL is a full URL.
        // We might need to bypass `ReqwestClient::builder` logic or use `reqwest::Client` directly but with `ReqwestClient`'s policies?
        // Or we can strip the endpoint from `upload_url` if it matches?

        // For now, I will use `self.inner.client` (legacy) for query if `ReqwestClient` doesn't support absolute URLs easily,
        // OR I can try to use `ReqwestClient` but I need to be careful.
        // `ReqwestClient` has `inner` which is `reqwest::Client`.
        // But I want tracing/auth from `ReqwestClient`.
        // `ReqwestClient::execute` takes a builder. I can create a builder from `self.inner.client` (or `self.inner.http_client.inner`) with absolute URL,
        // and then pass it to `execute`.
        // `ReqwestClient::execute` calls `configure_builder` which sets User-Agent and Host.
        // It also calls `request_attempt` which adds auth headers.

        // So I can do:
        let builder = self
            .inner
            .http_client
            .builder(reqwest::Method::PUT, upload_url.to_string())
            .header("content-type", "application/octet-stream")
            .header("Content-Range", "bytes */*")
            .header("content-length", 0)
            .header(
                "x-goog-api-client",
                reqwest::header::HeaderValue::from_static(&X_GOOG_API_CLIENT_HEADER),
            );

        // Now pass this builder to `self.inner.http_client.execute_streaming_once`.
        let response = self
            .inner
            .http_client
            .execute_streaming_once(builder, self.options.gax(), None, 0)
            .await?;

        self::query_resumable_upload_handle_response_v2(response).await
    }

    fn apply_preconditions(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let builder = self
            .spec
            .if_generation_match
            .iter()
            .fold(builder, |b, v| b.query(&[("ifGenerationMatch", v)]));
        let builder = self
            .spec
            .if_generation_not_match
            .iter()
            .fold(builder, |b, v| b.query(&[("ifGenerationNotMatch", v)]));
        let builder = self
            .spec
            .if_metageneration_match
            .iter()
            .fold(builder, |b, v| b.query(&[("ifMetagenerationMatch", v)]));
        let builder = self
            .spec
            .if_metageneration_not_match
            .iter()
            .fold(builder, |b, v| b.query(&[("ifMetagenerationNotMatch", v)]));

        [
            ("kmsKeyName", self.resource().kms_key.as_str()),
            ("predefinedAcl", self.spec.predefined_acl.as_str()),
        ]
        .into_iter()
        .fold(
            builder,
            |b, (k, v)| if v.is_empty() { b } else { b.query(&[(k, v)]) },
        )
    }
}

async fn handle_start_resumable_upload_response(response: reqwest::Response) -> Result<String> {
    if !response.status().is_success() {
        return gaxi::http::to_http_error(response).await;
    }
    let location = response
        .headers()
        .get("Location")
        .ok_or_else(|| Error::deser("missing Location header in start resumable upload"))?;
    location.to_str().map_err(Error::deser).map(str::to_string)
}

async fn query_resumable_upload_handle_response(
    response: reqwest::Response,
) -> Result<ResumableUploadStatus> {
    if response.status() == RESUME_INCOMPLETE {
        return self::parse_range(response).await;
    }
    let object = handle_object_response(response).await?;
    Ok(ResumableUploadStatus::Finalized(Box::new(object)))
}

async fn handle_object_response(response: reqwest::Response) -> Result<Object> {
    if !response.status().is_success() {
        return gaxi::http::to_http_error(response).await;
    }
    let response = response.json::<v1::Object>().await.map_err(Error::deser)?;
    Ok(Object::from(response))
}

async fn parse_range(response: reqwest::Response) -> Result<ResumableUploadStatus> {
    let Some(end) = self::parse_range_end(response.headers()) else {
        return gaxi::http::to_http_error(response).await;
    };
    // The `Range` header returns an inclusive range, i.e. bytes=0-999 means "1000 bytes".
    let persisted_size = match end {
        0 => 0,
        e => e + 1,
    };
    Ok(ResumableUploadStatus::Partial(persisted_size))
}

#[derive(Debug, PartialEq)]
enum ResumableUploadStatus {
    Finalized(Box<Object>),
    Partial(u64),
}

fn parse_range_end(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let Some(range) = headers.get("range") else {
        // A missing `Range:` header indicates that no bytes are persisted.
        return Some(0_u64);
    };
    // Uploads must be sequential, so the persisted range (if present) always
    // starts at zero. This is poorly documented, but can be inferred from
    //   https://cloud.google.com/storage/docs/performing-resumable-uploads#resume-upload
    // which requires uploads to continue from the last byte persisted. It is
    // better documented in the gRPC version, where holes are explicitly
    // forbidden:
    //   https://github.com/googleapis/googleapis/blob/302273adb3293bb504ecd83be8e1467511d5c779/google/storage/v2/storage.proto#L1253-L1255
    let end = std::str::from_utf8(range.as_bytes().strip_prefix(b"bytes=0-")?).ok()?;
    end.parse::<u64>().ok()
}

const RESUME_INCOMPLETE: reqwest::StatusCode = reqwest::StatusCode::PERMANENT_REDIRECT;

#[cfg(test)]
mod tests;

async fn handle_start_resumable_upload_response_v2<B>(
    response: gax::response::Response<B>,
) -> Result<String>
where
    B: futures::Stream<Item = Result<bytes::Bytes>> + Send + Unpin,
{
    let status_code = response
        .headers()
        .get("x-goog-status-code")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(200);
    if !reqwest::StatusCode::from_u16(status_code).unwrap_or(reqwest::StatusCode::OK).is_success() {
        return to_http_error_v2(response).await;
    }
    let location = response
        .headers()
        .get("Location")
        .ok_or_else(|| Error::deser("missing Location header in start resumable upload"))?;
    location.to_str().map_err(Error::deser).map(str::to_string)
}

async fn query_resumable_upload_handle_response_v2<B>(
    response: gax::response::Response<B>,
) -> Result<ResumableUploadStatus>
where
    B: futures::Stream<Item = Result<bytes::Bytes>> + Send + Unpin,
{
    let status_code = response
        .headers()
        .get("x-goog-status-code")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(200);
    if reqwest::StatusCode::from_u16(status_code).unwrap_or(reqwest::StatusCode::OK) == RESUME_INCOMPLETE {
        return self::parse_range_v2(response).await;
    }
    let object = handle_object_response_v2(response).await?;
    Ok(ResumableUploadStatus::Finalized(Box::new(object)))
}

async fn handle_object_response_v2<B>(response: gax::response::Response<B>) -> Result<Object>
where
    B: futures::Stream<Item = Result<bytes::Bytes>> + Send + Unpin,
{
    let status_code = response
        .headers()
        .get("x-goog-status-code")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(200);
    if !reqwest::StatusCode::from_u16(status_code).unwrap_or(reqwest::StatusCode::OK).is_success() {
        return to_http_error_v2(response).await;
    }
    use futures::TryStreamExt;
    let body = response.into_body().map_err(Error::io).try_fold(bytes::BytesMut::new(), |mut acc, chunk| async move {
        acc.extend_from_slice(&chunk);
        Ok(acc)
    }).await?;
    let object = serde_json::from_slice::<v1::Object>(&body).map_err(Error::deser)?;
    Ok(Object::from(object))
}

async fn parse_range_v2<B>(response: gax::response::Response<B>) -> Result<ResumableUploadStatus>
where
    B: futures::Stream<Item = Result<bytes::Bytes>> + Send + Unpin,
{
    let Some(end) = self::parse_range_end(response.headers()) else {
        return to_http_error_v2(response).await;
    };
    // The `Range` header returns an inclusive range, i.e. bytes=0-999 means "1000 bytes".
    let persisted_size = match end {
        0 => 0,
        e => e + 1,
    };
    Ok(ResumableUploadStatus::Partial(persisted_size))
}

async fn to_http_error_v2<B, O>(response: gax::response::Response<B>) -> Result<O>
where
    B: futures::Stream<Item = Result<bytes::Bytes>> + Send + Unpin,
{
    let headers = response.headers().clone();
    let status_code = headers
        .get("x-goog-status-code")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    use futures::TryStreamExt;
    let body = response.into_body().map_err(Error::io).try_fold(bytes::BytesMut::new(), |mut acc, chunk| async move {
        acc.extend_from_slice(&chunk);
        Ok(acc)
    }).await?;
    let body = body.freeze();

    let error = match gax::error::rpc::Status::try_from(&body) {
        Ok(status) => Error::service_with_http_metadata(status, Some(status_code), Some(headers)),
        Err(_) => Error::http(status_code, headers, body),
    };
    Err(error)
}
