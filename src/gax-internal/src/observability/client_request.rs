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

use crate::options::InstrumentationClientInfo;
use super::attributes::*;
use super::errors::ErrorType;
use gax::error::Error;
use gax::response::{Response, internal as response_internal};
use opentelemetry_semantic_conventions::{attribute as otel_attr, trace as otel_trace};
use tracing::{Level, Span};

// --- Client Request Span Helpers ---

/// Creates a Client Request Span.
#[allow(dead_code)]
pub fn create_client_request_span(
    name: &'static str,
    client_info: &'static InstrumentationClientInfo,
    _options: &gax::options::RequestOptions, // TODO: Use for gcp.resource_name
) -> Span {

    let span = tracing::span!(
        Level::INFO,
        "client_request",
        otel.name = name,
        { KEY_OTEL_KIND } = "Internal", // T2/T3 are Internal
        { KEY_GCP_CLIENT_SERVICE } = client_info.service_name,
        { KEY_GCP_CLIENT_VERSION } = client_info.client_version,
        { KEY_GCP_CLIENT_ARTIFACT } = client_info.client_artifact,
        { KEY_GCP_CLIENT_LANGUAGE } = GCP_CLIENT_LANGUAGE_RUST,
        { KEY_GCP_CLIENT_REPO } = GCP_CLIENT_REPO_RUST,
        // Attributes to be enriched later
        { KEY_OTEL_STATUS } = tracing::field::Empty,
        { otel_trace::HTTP_RESPONSE_STATUS_CODE } = tracing::field::Empty,
        { otel_attr::RPC_GRPC_STATUS_CODE } = tracing::field::Empty,
        { otel_trace::ERROR_TYPE } = tracing::field::Empty,
        { otel_trace::SERVER_ADDRESS } = tracing::field::Empty,
        { otel_trace::SERVER_PORT } = tracing::field::Empty,
        { otel_trace::URL_FULL } = tracing::field::Empty,
        { otel_trace::HTTP_REQUEST_RESEND_COUNT } = tracing::field::Empty,
    );
    // TODO: Add attributes from RequestOptions like gcp.resource_name
    span
}

/// Enriches the span with details from the response parts.
#[allow(dead_code)]
pub fn enrich_client_request_span<T>(response: &Response<T>, span: &Span) {

    if let Some(info) = response_internal::transport_span_info(response) {
        span.in_scope(|| {
            let current_span = Span::current();
            current_span.record(otel_trace::HTTP_RESPONSE_STATUS_CODE, info.http_status_code.map(|v| v as i64));
            current_span.record(otel_attr::RPC_GRPC_STATUS_CODE, info.rpc_grpc_status_code);
            current_span.record(otel_trace::ERROR_TYPE, info.error_type.as_deref());
            current_span.record(KEY_OTEL_STATUS, info.otel_status.as_deref());
            current_span.record(otel_trace::SERVER_ADDRESS, info.server_address.as_deref());
            current_span.record(otel_trace::SERVER_PORT, info.server_port);
            current_span.record(otel_trace::URL_FULL, info.url_full.as_deref());
            current_span.record(otel_trace::HTTP_REQUEST_RESEND_COUNT, info.request_resend_count);
        });
    }
}

/// Enriches the span with details from an error.
#[allow(dead_code)]
pub fn enrich_client_request_span_err(error: &Error, span: &Span) {
    span.in_scope(|| {
        let current_span = Span::current();
        let error_type = ErrorType::from(error);
        current_span.record(otel_trace::ERROR_TYPE, error_type.as_str());
        current_span.record(KEY_OTEL_STATUS, error_type.otel_status().as_str());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::InstrumentationClientInfo;
    use gax::options::RequestOptions;
    use gax::response::internal::{set_transport_span_info, TransportSpanInfo};
    use google_cloud_test_utils::test_layer::{AttributeValue, TestLayer};
    use std::collections::HashMap;
    use crate::observability::attributes::OtelStatus;

    const INFO: InstrumentationClientInfo = InstrumentationClientInfo {
        service_name: "test.service",
        client_version: "1.2.3",
        client_artifact: "google-cloud-test",
        default_host: "example.com",
    };

    #[tokio::test]
    async fn test_create_client_request_span() {
        let guard = TestLayer::initialize();
        let options = RequestOptions::default();
        let _span = create_client_request_span("test.method", &INFO, &options);

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        let event = &captured[0];
        assert_eq!(event.name, "client_request");

        let expected_attributes: HashMap<String, AttributeValue> = [
            ("otel.name".to_string(), "test.method".into()),
            ("otel.kind".to_string(), "Internal".into()),
            ("gcp.client.service".to_string(), "test.service".into()),
            ("gcp.client.version".to_string(), "1.2.3".into()),
            ("gcp.client.artifact".to_string(), "google-cloud-test".into()),
            ("gcp.client.language".to_string(), "rust".into()),
            ("gcp.client.repo".to_string(), "googleapis/google-cloud-rust".into()),
        ]
        .into_iter()
        .collect();
        assert_eq!(event.attributes, expected_attributes);
    }

    #[tokio::test]
    async fn test_enrich_client_request_span() {
        let guard = TestLayer::initialize();
        let options = RequestOptions::default();
        let span = create_client_request_span("test.method", &INFO, &options);
        let _enter = span.enter();

        let mut response = Response::from(true);
        let transport_info = TransportSpanInfo {
            http_status_code: Some(200),
            rpc_grpc_status_code: Some(0),
            error_type: None,
            otel_status: Some(OtelStatus::Ok.as_str().to_string()),
            server_address: Some("1.2.3.4".to_string()),
            server_port: Some(443),
            url_full: Some("https://example.com/test".to_string()),
            request_resend_count: Some(1),
        };
        set_transport_span_info(&mut response, Some(transport_info));

        enrich_client_request_span(&response, &span);

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        let event = &captured[0];

        let expected_attributes: HashMap<String, AttributeValue> = [
            ("otel.name".to_string(), "test.method".into()),
            ("otel.kind".to_string(), "Internal".into()),
            ("gcp.client.service".to_string(), "test.service".into()),
            ("gcp.client.version".to_string(), "1.2.3".into()),
            ("gcp.client.artifact".to_string(), "google-cloud-test".into()),
            ("gcp.client.language".to_string(), "rust".into()),
            ("gcp.client.repo".to_string(), "googleapis/google-cloud-rust".into()),
            // Enriched attributes
            ("http.response.status_code".to_string(), 200i64.into()),
            ("rpc.grpc.status_code".to_string(), 0i64.into()),
            ("server.address".to_string(), "1.2.3.4".into()),
            ("server.port".to_string(), 443i64.into()),
            ("url.full".to_string(), "https://example.com/test".into()),
            ("http.request.resend_count".to_string(), 1i64.into()),
            ("otel.status".to_string(), "Ok".into()),
        ]
        .into_iter()
        .collect();
        assert_eq!(event.attributes, expected_attributes);
    }

    #[tokio::test]
    async fn test_enrich_client_request_span_err() {
        let guard = TestLayer::initialize();
        let options = RequestOptions::default();
        let span = create_client_request_span("test.method", &INFO, &options);
        let _enter = span.enter();

        let error = Error::transport(http::HeaderMap::new(), "test error");
        enrich_client_request_span_err(&error, &span);

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        let event = &captured[0];

        let expected_attributes: HashMap<String, AttributeValue> = [
            ("otel.name".to_string(), "test.method".into()),
            ("otel.kind".to_string(), "Internal".into()),
            ("gcp.client.service".to_string(), "test.service".into()),
            ("gcp.client.version".to_string(), "1.2.3".into()),
            ("gcp.client.artifact".to_string(), "google-cloud-test".into()),
            ("gcp.client.language".to_string(), "rust".into()),
            ("gcp.client.repo".to_string(), "googleapis/google-cloud-rust".into()),
            // Enriched attributes
            ("error.type".to_string(), "CLIENT_CONNECTION_ERROR".into()),
            ("otel.status".to_string(), "Error".into()),
        ]
        .into_iter()
        .collect();
        assert_eq!(event.attributes, expected_attributes);
    }
}
