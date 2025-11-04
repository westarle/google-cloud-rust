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

use crate::observability::attributes::keys::*;
use crate::observability::attributes::*;
use crate::options::InstrumentationClientInfo;
use gax::options::RequestOptions;
use opentelemetry_semantic_conventions::{attribute as otel_attr, trace as otel_trace};
use tracing::{Span, field};

/// Creates a new tracing span for a logical client request (T3).
pub fn create_client_request_span(
    rpc_name: &str,
    _options: &RequestOptions,
    instrumentation: Option<&'static InstrumentationClientInfo>,
) -> Span {
    let (rpc_service, rpc_method) = match rpc_name.split_once('/') {
        Some((s, m)) => (s, m),
        None => ("unknown_service", rpc_name),
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
        "client_request",
        { OTEL_NAME } = rpc_name,
        { OTEL_KIND } = OTEL_KIND_CLIENT,
        { otel_trace::RPC_SYSTEM } = "google_cloud",
        { otel_trace::RPC_SERVICE } = rpc_service,
        { otel_trace::RPC_METHOD } = rpc_method,
        { otel_attr::URL_DOMAIN } = url_domain,
        { GCP_CLIENT_SERVICE } = gcp_client_service,
        { GCP_CLIENT_VERSION } = gcp_client_version,
        { GCP_CLIENT_REPO } = GCP_CLIENT_REPO_GOOGLEAPIS,
        { GCP_CLIENT_ARTIFACT } = gcp_client_artifact,
        { GCP_CLIENT_LANGUAGE } = GCP_CLIENT_LANGUAGE_RUST,
        // Fields to be recorded later
        { OTEL_STATUS_CODE } = otel_status_codes::UNSET,
        { OTEL_STATUS_DESCRIPTION } = field::Empty,
        { otel_trace::HTTP_RESPONSE_STATUS_CODE } = field::Empty,
        { otel_attr::RPC_GRPC_STATUS_CODE } = field::Empty,
        { GRPC_STATUS } = field::Empty,
        { otel_trace::ERROR_TYPE } = field::Empty,
        { otel_trace::SERVER_ADDRESS } = field::Empty,
        { otel_trace::SERVER_PORT } = field::Empty,
        { otel_trace::URL_FULL } = field::Empty,
        { otel_trace::HTTP_REQUEST_RESEND_COUNT } = field::Empty,
    )
}

/// Enriches the client request span with information from the transport layer.
pub fn enrich_client_request_span(
    span: &Span,
    transport_info: Option<&gax::response::internal::TransportSpanInfo>,
) {
    if let Some(info) = transport_info {
        if let Some(status) = info.http_status_code {
            span.record(otel_trace::HTTP_RESPONSE_STATUS_CODE, status as i64);
        }
        if let Some(status) = info.rpc_grpc_status_code {
            span.record(otel_attr::RPC_GRPC_STATUS_CODE, status as i64);
            // Also set grpc.status for backward compatibility if needed, or just use one.
            // Using both for now as defined in keys.
            span.record(GRPC_STATUS, status as i64);
        }
        if let Some(error_type) = &info.error_type {
            span.record(otel_trace::ERROR_TYPE, error_type.as_str());
            span.record(OTEL_STATUS_CODE, otel_status_codes::ERROR);
        } else {
             span.record(OTEL_STATUS_CODE, otel_status_codes::OK);
        }

        if let Some(address) = &info.server_address {
            span.record(otel_trace::SERVER_ADDRESS, address.as_str());
        }
        if let Some(port) = info.server_port {
            span.record(otel_trace::SERVER_PORT, port as i64);
        }
        if let Some(url) = &info.url_full {
            span.record(otel_trace::URL_FULL, url.as_str());
        }
        if let Some(count) = info.request_resend_count {
            span.record(otel_trace::HTTP_REQUEST_RESEND_COUNT, count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::InstrumentationClientInfo;
    use gax::options::RequestOptions;
    use google_cloud_test_utils::test_layer::{AttributeValue, TestLayer};
    use std::collections::HashMap;

    const TEST_INFO: InstrumentationClientInfo = InstrumentationClientInfo {
        service_name: "test.service",
        client_version: "1.2.3",
        client_artifact: "google-cloud-test",
        default_host: "test.googleapis.com",
    };

    #[tokio::test]
    async fn test_create_client_request_span() {
        let guard = TestLayer::initialize();
        let options = RequestOptions::default();
        let rpc_name = "google.cloud.test.v1.TestService/TestMethod";

        let _span = create_client_request_span(rpc_name, &options, Some(&TEST_INFO));

        let expected_attributes: HashMap<String, AttributeValue> = [
            (OTEL_NAME, rpc_name.into()),
            (OTEL_KIND, OTEL_KIND_CLIENT.into()),
            (otel_trace::RPC_SYSTEM, "google_cloud".into()),
            (otel_trace::RPC_SERVICE, "google.cloud.test.v1.TestService".into()),
            (otel_trace::RPC_METHOD, "TestMethod".into()),
            (otel_attr::URL_DOMAIN, "test.googleapis.com".into()),
            (GCP_CLIENT_SERVICE, "test.service".into()),
            (GCP_CLIENT_VERSION, "1.2.3".into()),
            (GCP_CLIENT_REPO, GCP_CLIENT_REPO_GOOGLEAPIS.into()),
            (GCP_CLIENT_ARTIFACT, "google-cloud-test".into()),
            (GCP_CLIENT_LANGUAGE, GCP_CLIENT_LANGUAGE_RUST.into()),
            (OTEL_STATUS_CODE, otel_status_codes::UNSET.into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].attributes, expected_attributes);
    }

    #[tokio::test]
    async fn test_enrich_client_request_span() {
        let guard = TestLayer::initialize();
        let options = RequestOptions::default();
        let span = create_client_request_span("test/method", &options, None);
        let _enter = span.enter();

        let transport_info = gax::response::internal::TransportSpanInfo {
            http_status_code: Some(200),
            server_address: Some("1.2.3.4".to_string()),
            server_port: Some(443),
            url_full: Some("https://test.googleapis.com/v1/resource".to_string()),
            request_resend_count: Some(1),
            ..Default::default()
        };

        enrich_client_request_span(&span, Some(&transport_info));

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        let attributes = &captured[0].attributes;

        assert_eq!(
            attributes.get(otel_trace::HTTP_RESPONSE_STATUS_CODE),
            Some(&200_i64.into())
        );
        assert_eq!(
            attributes.get(otel_trace::SERVER_ADDRESS),
            Some(&"1.2.3.4".into())
        );
        assert_eq!(
            attributes.get(otel_trace::SERVER_PORT),
            Some(&443_i64.into())
        );
        assert_eq!(
            attributes.get(otel_trace::URL_FULL),
            Some(&"https://test.googleapis.com/v1/resource".into())
        );
        assert_eq!(
            attributes.get(otel_trace::HTTP_REQUEST_RESEND_COUNT),
            Some(&1_i64.into())
        );
        assert_eq!(
            attributes.get(OTEL_STATUS_CODE),
            Some(&otel_status_codes::OK.into())
        );
    }

    #[tokio::test]
    async fn test_enrich_client_request_span_error() {
        let guard = TestLayer::initialize();
        let options = RequestOptions::default();
        let span = create_client_request_span("test/method", &options, None);
        let _enter = span.enter();

        let transport_info = gax::response::internal::TransportSpanInfo {
            http_status_code: Some(404),
            error_type: Some("404".to_string()),
            rpc_grpc_status_code: Some(5), // NOT_FOUND
            ..Default::default()
        };

        enrich_client_request_span(&span, Some(&transport_info));

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        let attributes = &captured[0].attributes;

        assert_eq!(
            attributes.get(otel_trace::HTTP_RESPONSE_STATUS_CODE),
            Some(&404_i64.into())
        );
        assert_eq!(
            attributes.get(otel_trace::ERROR_TYPE),
            Some(&"404".into())
        );
        assert_eq!(
            attributes.get(otel_attr::RPC_GRPC_STATUS_CODE),
            Some(&5_i64.into())
        );
        assert_eq!(
            attributes.get(OTEL_STATUS_CODE),
            Some(&otel_status_codes::ERROR.into())
        );
    }
}
