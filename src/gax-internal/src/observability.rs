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

use crate::http::ReqwestClient;
use crate::options::InstrumentationClientInfo;
use gax::options::RequestOptions;

#[derive(Debug, Clone)]
// TODO(#3239): Remove once used in http.rs
#[allow(dead_code)]
pub(crate) struct HttpSpanInfo {
    // Kept Attributes for OpenTelemetry SDK interop
    rpc_system: &'static str, // "http"
    otel_kind: &'static str,  // "Client"
    otel_name: String,        // "{METHOD} {url.template}" or "{METHOD}"
    otel_status: &'static str, // "Unset", "Ok", "Error"

    // OTel Standard HTTP Attributes
    http_request_method: String,
    server_address: Option<String>, // Host from URL
    server_port: Option<u16>,      // Port from URL
    url_full: String,
    url_scheme: Option<String>,
    url_template: Option<String>, // From RequestOptions.path_template

    // Domain from InstrumentationClientInfo (intended host)
    url_domain: Option<&'static str>,

    http_response_status_code: Option<u16>,
    error_type: Option<&'static str>,
    http_request_resend_count: Option<u32>, // options.prior_attempt_count (only if > 0)

    // Custom GCP Attributes
    gcp_client_service: Option<&'static str>,
    gcp_client_version: Option<&'static str>,
    gcp_client_repo: &'static str, // "googleapis/google-cloud-rust"
    gcp_client_artifact: Option<&'static str>,
}

impl HttpSpanInfo {
    // TODO(#3239): Remove once used in http.rs
    #[allow(dead_code)]
    pub(crate) fn new(
        _client: &ReqwestClient,
        request: &reqwest::Request,
        options: &RequestOptions,
        instrumentation: Option<&InstrumentationClientInfo>,
        current_attempt: u32,
    ) -> Self {
        let url = request.url();
        let method = request.method();

        let url_template = gax::options::internal::get_path_template(options);
        let otel_name = url_template.map_or_else(
            || method.to_string(),
            |template| format!("{} {}", method, template),
        );

        let http_request_resend_count = if current_attempt > 0 {
            Some(current_attempt)
        } else {
            None
        };

        let (gcp_client_service, gcp_client_version, gcp_client_artifact, url_domain) =
            instrumentation.map_or((None, None, None, None), |info| {
                (
                    Some(info.service_name),
                    Some(info.client_version),
                    Some(info.client_artifact),
                    Some(info.default_host),
                )
            });

        Self {
            rpc_system: "http",
            otel_kind: "Client",
            otel_name,
            otel_status: "Unset",
            http_request_method: method.to_string(),
            server_address: url.host_str().map(String::from),
            server_port: url.port_or_known_default(),
            url_full: url.to_string(),
            url_scheme: Some(url.scheme().to_string()),
            url_template: url_template.map(String::from),
            url_domain,
            http_response_status_code: None,
            error_type: None,
            http_request_resend_count,
            gcp_client_service,
            gcp_client_version,
            gcp_client_repo: "googleapis/google-cloud-rust",
            gcp_client_artifact,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gax::options::RequestOptions;
    use crate::http::ReqwestClient;
    use crate::options::{ClientConfig, InstrumentationClientInfo};
    use http::Method;
    use reqwest;

    // Helper to create a dummy ReqwestClient
    async fn dummy_client() -> ReqwestClient {
        let config = ClientConfig::default();
        ReqwestClient::new(config, "https://example.com").await.unwrap()
    }

    #[tokio::test]
    async fn test_http_span_info_new_basic() {
        let client = dummy_client().await;
        let request = reqwest::Request::new(Method::GET, "https://example.com/test".parse().unwrap());
        let options = RequestOptions::default();

        let span_info = HttpSpanInfo::new(&client, &request, &options, None, 0);

        assert_eq!(span_info.rpc_system, "http");
        assert_eq!(span_info.otel_kind, "Client");
        assert_eq!(span_info.otel_name, "GET");
        assert_eq!(span_info.otel_status, "Unset");
        assert_eq!(span_info.http_request_method, "GET");
        assert_eq!(span_info.server_address, Some("example.com".to_string()));
        assert_eq!(span_info.server_port, Some(443)); // Default port for https
        assert_eq!(span_info.url_full, "https://example.com/test");
        assert_eq!(span_info.url_scheme, Some("https".to_string()));
        assert_eq!(span_info.url_template, None);
        assert_eq!(span_info.url_domain, None);
        assert_eq!(span_info.http_response_status_code, None);
        assert_eq!(span_info.error_type, None);
        assert_eq!(span_info.http_request_resend_count, None);
        assert_eq!(span_info.gcp_client_service, None);
        assert_eq!(span_info.gcp_client_version, None);
        assert_eq!(span_info.gcp_client_repo, "googleapis/google-cloud-rust");
        assert_eq!(span_info.gcp_client_artifact, None);
    }

    #[tokio::test]
    async fn test_http_span_info_new_with_instrumentation() {
        let client = dummy_client().await;
        let request = reqwest::Request::new(Method::POST, "https://test.service.dev:443/v1/items".parse().unwrap());
        let options = RequestOptions::default();
        const INFO: InstrumentationClientInfo = InstrumentationClientInfo {
            service_name: "test.service",
            client_version: "1.2.3",
            client_artifact: "google-cloud-test",
            default_host: "test.service.dev",
        };

        let span_info = HttpSpanInfo::new(&client, &request, &options, Some(&INFO), 0);

        assert_eq!(span_info.gcp_client_service, Some("test.service"));
        assert_eq!(span_info.gcp_client_version, Some("1.2.3"));
        assert_eq!(span_info.gcp_client_artifact, Some("google-cloud-test"));
        assert_eq!(span_info.url_domain, Some("test.service.dev"));
        assert_eq!(span_info.server_address, Some("test.service.dev".to_string()));
        assert_eq!(span_info.server_port, Some(443));
    }

    #[tokio::test]
    async fn test_http_span_info_new_with_path_template() {
        let client = dummy_client().await;
        let request = reqwest::Request::new(Method::GET, "https://example.com/items/123".parse().unwrap());
        let options = gax::options::internal::set_path_template(
            RequestOptions::default(),
            Some("/items/{item_id}".to_string()),
        );

        let span_info = HttpSpanInfo::new(&client, &request, &options, None, 0);

        assert_eq!(span_info.url_template, Some("/items/{item_id}".to_string()));
        assert_eq!(span_info.otel_name, "GET /items/{item_id}");
    }

    #[tokio::test]
    async fn test_http_span_info_new_with_attempt_count() {
        let client = dummy_client().await;
        let request = reqwest::Request::new(Method::GET, "https://example.com/test".parse().unwrap());
        let options = RequestOptions::default();

        // current_attempt is 0 for the first try
        let span_info = HttpSpanInfo::new(&client, &request, &options, None, 0);
        assert_eq!(span_info.http_request_resend_count, None);

        // current_attempt is 1 for the second try (first retry)
        let span_info = HttpSpanInfo::new(&client, &request, &options, None, 1);
        assert_eq!(span_info.http_request_resend_count, Some(1));

        let span_info = HttpSpanInfo::new(&client, &request, &options, None, 5);
        assert_eq!(span_info.http_request_resend_count, Some(5));
    }
}
