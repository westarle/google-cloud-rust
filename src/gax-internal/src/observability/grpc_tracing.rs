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
use opentelemetry_semantic_conventions::{attribute as otel_attr, trace as otel_trace};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::Instrument;

/// A Tower layer that adds structured tracing to gRPC requests that is compatible with OpenTelemetry.
///
/// This layer is responsible for wrapping the inner service with a
/// [`TracingTowerService`], which intercepts requests and creates tracing spans.
///
/// It is typically used with [`tower::ServiceBuilder`] to add tracing middleware
/// to a gRPC client.
#[derive(Clone, Debug, Default)]
pub struct TracingTowerLayer {
    endpoint: String,
}

impl TracingTowerLayer {
    /// Creates a new `TracingTowerLayer`.
    pub fn new(endpoint: String) -> Self {
        Self { endpoint }
    }
}

impl<S> Layer<S> for TracingTowerLayer {
    type Service = TracingTowerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TracingTowerService {
            inner,
            layer: self.clone(),
        }
    }
}

/// A Tower service that intercepts gRPC requests to create tracing spans.
///
/// This service wraps an inner service and instruments the returned future with
/// a tracing span. The span is named "grpc.request" and is created at the `INFO`
/// level.
#[derive(Clone, Debug)]
pub struct TracingTowerService<S> {
    inner: S,
    layer: TracingTowerLayer,
}

impl<S, B, ResBody> Service<http::Request<B>> for TracingTowerService<S>
where
    S: Service<http::Request<B>, Response = http::Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display,
{
    type Response = S::Response;
    type Error = S::Error;
    // We use `Box<dyn Future...>` (type erasure) here to simplify the type signature.
    // Without this, we would need to explicitly name the complex type returned by
    // `.instrument()` (and any implementation changes in `call`), which can be verbose and brittle.
    //
    // The allocation cost is negligible as `call` is invoked once per RPC (or stream initialization),
    // not per message in a streaming call.
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        let span = create_grpc_span(req.uri(), &self.layer.endpoint);
        Box::pin(self.inner.call(req).instrument(span))
    }
}

fn create_grpc_span(uri: &http::Uri, endpoint: &str) -> tracing::Span {
    let (rpc_service, rpc_method) = parse_method(uri.path());
    let (server_address, server_port, url_domain) = parse_endpoint(endpoint);
    let span_name = format!("{}/{}", rpc_service, rpc_method);
    tracing::info_span!(
        "grpc.request",
        { OTEL_NAME } = %span_name,
        { otel_trace::RPC_SYSTEM } = "grpc",
        { OTEL_KIND } = crate::observability::attributes::OTEL_KIND_CLIENT,
        { otel_trace::RPC_SERVICE } = %rpc_service,
        { otel_trace::RPC_METHOD } = %rpc_method,
        { otel_trace::SERVER_ADDRESS } = %server_address,
        { otel_trace::SERVER_PORT } = server_port.map(|p| p as i64),
        { otel_attr::URL_DOMAIN } = %url_domain,
        // Standard attributes that will be populated later
        { otel_attr::RPC_GRPC_STATUS_CODE } = tracing::field::Empty,
        { GRPC_STATUS } = tracing::field::Empty,
        { OTEL_STATUS_CODE } = tracing::field::Empty,
        { otel_trace::ERROR_TYPE } = tracing::field::Empty,
    )
}

fn parse_method(path: &str) -> (String, String) {
    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() == 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        ("unknown".to_string(), "unknown".to_string())
    }
}

fn parse_endpoint(endpoint: &str) -> (String, Option<u16>, String) {
    // The endpoint is typically "https://service.googleapis.com".
    // We need to parse it to get the host and port.
    // If parsing fails, we fallback to the raw string.
    if let Ok(uri) = endpoint.parse::<http::Uri>() {
        let host = uri.host().unwrap_or(endpoint).to_string();
        let port = uri.port_u16().or_else(|| match uri.scheme_str() {
            Some("https") => Some(443),
            Some("http") => Some(80),
            _ => None,
        });
        (host.clone(), port, host)
    } else {
        (endpoint.to_string(), None, endpoint.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use google_cloud_test_utils::test_layer::{AttributeValue, TestLayer};
    use std::collections::HashMap;

    #[test]
    fn test_parse_method() {
        assert_eq!(
            parse_method("/google.pubsub.v1.Publisher/Publish"),
            (
                "google.pubsub.v1.Publisher".to_string(),
                "Publish".to_string()
            )
        );
        assert_eq!(
            parse_method("google.pubsub.v1.Publisher/Publish"),
            (
                "google.pubsub.v1.Publisher".to_string(),
                "Publish".to_string()
            )
        );
        assert_eq!(
            parse_method("/invalid/path/format"),
            ("unknown".to_string(), "unknown".to_string())
        );
        assert_eq!(
            parse_method("invalid"),
            ("unknown".to_string(), "unknown".to_string())
        );
    }

    #[test]
    fn test_parse_endpoint() {
        assert_eq!(
            parse_endpoint("https://pubsub.googleapis.com"),
            ("pubsub.googleapis.com".to_string(), Some(443), "pubsub.googleapis.com".to_string())
        );
        assert_eq!(
            parse_endpoint("http://localhost:8080"),
            ("localhost".to_string(), Some(8080), "localhost".to_string())
        );
        assert_eq!(
            parse_endpoint("http://127.0.0.1:9090"),
            ("127.0.0.1".to_string(), Some(9090), "127.0.0.1".to_string())
        );
        assert_eq!(
            parse_endpoint("pubsub.googleapis.com"), // No scheme, might fail parsing or return as is
            ("pubsub.googleapis.com".to_string(), None, "pubsub.googleapis.com".to_string())
        );
        assert_eq!(
            parse_endpoint("invalid uri"), 
            ("invalid uri".to_string(), None, "invalid uri".to_string())
        );
    }

    #[test]
    fn test_create_grpc_span() {
        let guard = TestLayer::initialize();
        let uri = http::Uri::from_static("https://pubsub.googleapis.com/google.pubsub.v1.Publisher/Publish");
        let endpoint = "https://pubsub.googleapis.com";
        let _span = create_grpc_span(&uri, endpoint);

        let captured = TestLayer::capture(&guard);
        assert_eq!(captured.len(), 1);
        let span = &captured[0];
        assert_eq!(span.name, "grpc.request");

        let expected_attributes: HashMap<String, AttributeValue> = [
            (OTEL_NAME, "google.pubsub.v1.Publisher/Publish".into()),
            (otel_trace::RPC_SYSTEM, "grpc".into()),
            (OTEL_KIND, "Client".into()),
            (otel_trace::RPC_SERVICE, "google.pubsub.v1.Publisher".into()),
            (otel_trace::RPC_METHOD, "Publish".into()),
            (otel_trace::SERVER_ADDRESS, "pubsub.googleapis.com".into()),
            (otel_trace::SERVER_PORT, 443_i64.into()),
            (otel_attr::URL_DOMAIN, "pubsub.googleapis.com".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        assert_eq!(span.attributes, expected_attributes);
    }
}
