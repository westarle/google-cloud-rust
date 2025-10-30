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

use crate::observability::attributes::*;
use crate::observability::errors::ErrorType;
use crate::options::InstrumentationClientInfo;
use gax::options::RequestOptions;
use opentelemetry_semantic_conventions::{attribute as otel_attr, trace as otel_trace};
use tracing::{Span, field};

/// Creates a new tracing span for an HTTP request attempt.
///
/// Populates the span with attributes available before the request is sent,
/// adhering to OpenTelemetry semantic conventions.
pub(crate) fn create_http_attempt_span(
    request: &reqwest::Request,
    options: &RequestOptions,
    instrumentation: Option<&'static InstrumentationClientInfo>,
    prior_attempt_count: u32,
) -> Span {
    let url = request.url();
    let method = request.method();

    let url_template = gax::options::internal::get_path_template(options);
    let otel_name = url_template.map_or_else(
        || method.to_string(),
        |template| format!("{} {}", method, template),
    );

    let http_request_resend_count = if prior_attempt_count > 0 {
        Some(prior_attempt_count as i64)
    } else {
        None
    };

    let (gcp_client_service, gcp_client_version, gcp_client_artifact, url_domain) = instrumentation
        .map_or((None, None, None, None), |info| {
            (
                Some(info.service_name),
                Some(info.client_version),
                Some(info.client_artifact),
                Some(info.default_host),
            )
        });

    tracing::info_span!(
        "http_request",
        { KEY_OTEL_NAME } = otel_name,
        { KEY_OTEL_KIND } = "Client",
        { otel_trace::RPC_SYSTEM } = "http",
        { otel_trace::HTTP_REQUEST_METHOD } = method.as_str(),
        { otel_trace::SERVER_ADDRESS } = url
            .host_str()
            .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
            .unwrap_or(""),
        { otel_trace::SERVER_PORT } = url.port_or_known_default().map(|p| p as i64).unwrap_or(0),
        { otel_trace::URL_FULL } = url.as_str(),
        { otel_trace::URL_SCHEME } = url.scheme(),
        { otel_attr::URL_TEMPLATE } = url_template,
        { otel_attr::URL_DOMAIN } = url_domain,
        { KEY_GCP_CLIENT_SERVICE } = gcp_client_service,
        { KEY_GCP_CLIENT_VERSION } = gcp_client_version,
        { KEY_GCP_CLIENT_REPO } = "googleapis/google-cloud-rust",
        { KEY_GCP_CLIENT_ARTIFACT } = gcp_client_artifact,
        { otel_trace::HTTP_REQUEST_RESEND_COUNT } = http_request_resend_count,
        // Fields to be recorded later
        { KEY_OTEL_STATUS } = OtelStatus::Unset.as_str(), // Initial state
        { otel_trace::HTTP_RESPONSE_STATUS_CODE } = field::Empty,
        { otel_trace::ERROR_TYPE } = field::Empty,
        { otel_attr::RPC_GRPC_STATUS_CODE } = field::Empty,
        { KEY_GRPC_STATUS } = field::Empty,
    )
}

/// Records additional attributes to the span based on the response outcome.
pub(crate) fn record_http_response_attributes(
    span: &Span,
    result: &Result<reqwest::Response, reqwest::Error>,
) {
    match result {
        Ok(response) => {
            let status = response.status();
            span.record(
                otel_trace::HTTP_RESPONSE_STATUS_CODE,
                status.as_u16() as i64,
            );
            if status.is_success() {
                span.record(KEY_OTEL_STATUS, OtelStatus::Ok.as_str());
            } else {
                span.record(KEY_OTEL_STATUS, OtelStatus::Error.as_str());
                // TODO(#3239): Extract reason from response headers/body if available
                let error_type = ErrorType::HttpError {
                    code: status,
                    reason: None,
                };
                span.record(otel_trace::ERROR_TYPE, error_type.as_str());
                span.record(
                    otel_attr::RPC_GRPC_STATUS_CODE,
                    error_type.grpc_code() as i64,
                );
                span.record(KEY_GRPC_STATUS, error_type.grpc_status());
            }
        }
        Err(err) => {
            span.record(KEY_OTEL_STATUS, OtelStatus::Error.as_str());
            let error_type = ErrorType::from_reqwest_error(err);
            span.record(otel_trace::ERROR_TYPE, error_type.as_str());
            span.record(
                otel_attr::RPC_GRPC_STATUS_CODE,
                error_type.grpc_code() as i64,
            );
            span.record(KEY_GRPC_STATUS, error_type.grpc_status());
        }
    }
}

/// Creates a TransportSpanInfo from the result of an HTTP request.
#[cfg(google_cloud_unstable_tracing)]
pub(crate) fn create_transport_span_info(
    result: &Result<reqwest::Response, reqwest::Error>,
    attempt_count: u32,
) -> gax::response::internal::TransportSpanInfo {
    let mut info = gax::response::internal::TransportSpanInfo::default();
    info.request_resend_count = if attempt_count > 1 {
        Some((attempt_count - 1) as i64)
    } else {
        None
    };

    match result {
        Ok(response) => {
            info.http_status_code = Some(response.status().as_u16());
            info.url_full = Some(response.url().to_string());
            if let Some(remote_addr) = response.remote_addr() {
                info.server_address = Some(remote_addr.ip().to_string());
                info.server_port = Some(remote_addr.port() as i32);
            }

            if !response.status().is_success() {
                let error_type = ErrorType::HttpError {
                    code: response.status(),
                    reason: None, // TODO: Extract from body if possible
                };
                info.error_type = Some(error_type.as_str());
                info.rpc_grpc_status_code = Some(error_type.grpc_code() as i32);
            }
        }
        Err(err) => {
            let error_type = ErrorType::from_reqwest_error(err);
            info.error_type = Some(error_type.as_str());
            info.rpc_grpc_status_code = Some(error_type.grpc_code() as i32);
            if let Some(url) = err.url() {
                info.url_full = Some(url.to_string());
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
}
