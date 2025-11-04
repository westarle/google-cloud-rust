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

use crate::options::{ClientConfig, InstrumentationClientInfo};

/// Holds information required to create and finalize a gRPC network span.
#[derive(Debug)]
pub(crate) struct GrpcSpanInfo {
    pub rpc_service: String,
    pub rpc_method: String,
    pub server_address: String,
    pub server_port: u16,
    pub url_domain: String,
    pub client_info: Option<&'static InstrumentationClientInfo>,
}

impl GrpcSpanInfo {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rpc_service: String,
        rpc_method: String,
        server_address: String,
        server_port: u16,
        url_domain: String,
        client_info: Option<&'static InstrumentationClientInfo>,
    ) -> Self {
        Self {
            rpc_service,
            rpc_method,
            server_address,
            server_port,
            url_domain,
            client_info,
        }
    }
}

#[cfg(test)]
mod span_info_tests {
    use super::*;
    use crate::options::InstrumentationClientInfo;

    #[test]
    fn test_grpc_span_info_new() {
        static TEST_INFO: InstrumentationClientInfo = InstrumentationClientInfo {
            service_name: "test-service",
            client_version: "1.0.0",
            client_artifact: "test-artifact",
            default_host: "example.com",
        };

        let span_info = GrpcSpanInfo::new(
            "my.service".to_string(),
            "MyMethod".to_string(),
            "example.com".to_string(),
            443,
            "example.com".to_string(),
            Some(&TEST_INFO),
        );

        assert_eq!(span_info.rpc_service, "my.service");
        assert_eq!(span_info.rpc_method, "MyMethod");
        assert_eq!(span_info.server_address, "example.com");
        assert_eq!(span_info.server_port, 443);
        assert_eq!(span_info.url_domain, "example.com");
        assert!(span_info.client_info.is_some());
    }
}

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing::{Instrument, Span, warn};

#[derive(Clone, Debug)]
pub struct GrpcTowerLayer {
    pub(crate) config: ClientConfig,
    pub(crate) client_info: Option<&'static InstrumentationClientInfo>,
    pub(crate) server_address: String,
    pub(crate) server_port: u16,
    pub(crate) url_domain: String,
}

impl GrpcTowerLayer {
    pub fn new(
        config: ClientConfig,
        server_address: String,
        server_port: u16,
        url_domain: String,
        client_info: Option<&'static InstrumentationClientInfo>,
    ) -> Self {
        Self {
            config,
            client_info,
            server_address,
            server_port,
            url_domain,
        }
    }
}

impl<S> Layer<S> for GrpcTowerLayer {
    type Service = GrpcTowerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcTowerService {
            inner,
            layer: self.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GrpcTowerService<S> {
    inner: S,
    layer: GrpcTowerLayer,
}

impl<S, B> Service<http::Request<B>> for GrpcTowerService<S>
where
    S: Service<http::Request<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        if !crate::options::tracing_enabled(&self.layer.config) {
            return Box::pin(self.inner.call(req));
        }

        let path = req.uri().path();
        // path is something like /google.pubsub.v1.Publisher/Publish
        let mut parts = path.split('/');
        parts.next(); // skip empty part before first '/'
        let rpc_service = parts.next().unwrap_or_default().to_string();
        let rpc_method = parts.next().unwrap_or_default().to_string();

        if rpc_service.is_empty() || rpc_method.is_empty() {
            warn!(
                "Failed to parse RPC service and method from URI path: {}",
                path
            );
            return Box::pin(self.inner.call(req));
        }

        let span_info = GrpcSpanInfo::new(
            rpc_service,
            rpc_method,
            self.layer.server_address.clone(),
            self.layer.server_port,
            self.layer.url_domain.clone(),
            self.layer.client_info,
        );

        let span = tracing::info_span!(
            "grpc.request",
            otel.name = format!("{}/{}", span_info.rpc_service, span_info.rpc_method).as_str(),
            otel.kind = "Client",
            rpc.system = "grpc",
            rpc.service = span_info.rpc_service.as_str(),
            rpc.method = span_info.rpc_method.as_str(),
            server.address = span_info.server_address.as_str(),
            server.port = span_info.server_port,
            url.domain = span_info.url_domain.as_str(),
            rpc.grpc.status_code = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
            gcp.client.service = tracing::field::Empty,
            gcp.client.version = tracing::field::Empty,
            gcp.client.repo = tracing::field::Empty,
            gcp.client.artifact = tracing::field::Empty,
        );

        if let Some(client_info) = span_info.client_info {
            span.record("gcp.client.service", client_info.service_name);
            span.record("gcp.client.version", client_info.client_version);
            span.record("gcp.client.repo", "googleapis/google-cloud-rust");
            span.record("gcp.client.artifact", client_info.client_artifact);
        }

        let future = self.inner.call(req).instrument(span.clone());

        Box::pin(ResponseFuture {
            inner: future,
            span,
            span_info,
        })
    }
}

#[pin_project::pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    span: Span,
    span_info: GrpcSpanInfo,
}

impl<F, Response, Error> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response, Error>>,
    Error: std::fmt::Display,
{
    type Output = Result<Response, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();
        let result = futures_util::ready!(this.inner.poll(cx));

        match &result {
            Ok(_) => {
                this.span
                    .record("rpc.grpc.status_code", tonic::Code::Ok as i32);
                this.span.record("otel.status_code", "OK");
            }
            Err(e) => {
                // TODO: Try to extract tonic::Status from error if possible
                warn!("gRPC request failed: {}", e);
                this.span.record("otel.status_code", "ERROR");
            }
        }
        Poll::Ready(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{ClientConfig, InstrumentationClientInfo};
    use bytes::Bytes;
    use google_cloud_test_utils::test_layer::TestLayer;
    use http::{Request, Uri};
    use http_body_util::{BodyExt, Empty, combinators::UnsyncBoxBody};
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tower::{Layer, Service};

    #[test]
    fn test_grpc_tower_layer() {
        static TEST_INFO: InstrumentationClientInfo = InstrumentationClientInfo {
            service_name: "test-service",
            client_version: "1.0.0",
            client_artifact: "test-artifact",
            default_host: "example.com",
        };
        let layer = GrpcTowerLayer::new(
            ClientConfig::default(),
            "example.com".to_string(),
            443,
            "example.com".to_string(),
            Some(&TEST_INFO),
        );
        assert!(layer.client_info.is_some());
        assert_eq!(layer.server_address, "example.com");
    }

    #[test]
    fn test_grpc_tower_service() {
        // Dummy service for testing
        #[derive(Clone)]
        struct DummyService;
        impl Service<Request<UnsyncBoxBody<Bytes, tonic::Status>>> for DummyService {
            type Response = http::Response<UnsyncBoxBody<Bytes, tonic::Status>>;
            type Error = tonic::transport::Error;
            type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: Request<UnsyncBoxBody<Bytes, tonic::Status>>) -> Self::Future {
                Box::pin(async {
                    Ok(http::Response::new(
                        Empty::<Bytes>::new()
                            .map_err(|_| tonic::Status::unknown("unreachable"))
                            .boxed_unsync(),
                    ))
                })
            }
        }

        let layer = GrpcTowerLayer::new(
            ClientConfig::default(),
            "example.com".to_string(),
            443,
            "example.com".to_string(),
            None,
        );
        let service = layer.layer(DummyService);
        assert!(service.layer.client_info.is_none());
    }

    #[derive(Clone)]
    struct DummyService;
    impl Service<Request<UnsyncBoxBody<Bytes, tonic::Status>>> for DummyService {
        type Response = http::Response<UnsyncBoxBody<Bytes, tonic::Status>>;
        type Error = tonic::transport::Error;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, _req: Request<UnsyncBoxBody<Bytes, tonic::Status>>) -> Self::Future {
            Box::pin(async {
                Ok(http::Response::new(
                    Empty::<Bytes>::new()
                        .map_err(|_| tonic::Status::unknown("unreachable"))
                        .boxed_unsync(),
                ))
            })
        }
    }

    #[tokio::test]
    async fn test_grpc_span_attributes_with_client_info() {
        let guard = TestLayer::initialize();

        static TEST_INFO: InstrumentationClientInfo = InstrumentationClientInfo {
            service_name: "test-service",
            client_version: "1.0.0",
            client_artifact: "test-artifact",
            default_host: "example.com",
        };

        let mut config = ClientConfig::default();
        config.tracing = true;

        let layer = GrpcTowerLayer::new(
            config,
            "example.com".to_string(),
            443,
            "example.com".to_string(),
            Some(&TEST_INFO),
        );
        let mut service = layer.layer(DummyService);

        let req = Request::builder()
            .uri(Uri::from_static(
                "https://example.com/google.pubsub.v1.Publisher/Publish",
            ))
            .body(
                Empty::<Bytes>::new()
                    .map_err(|_| tonic::Status::unknown(""))
                    .boxed_unsync(),
            )
            .unwrap();

        let _ = service.call(req).await;

        let spans = TestLayer::capture(&guard);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];

        assert_eq!(span.name, "grpc.request");
        assert_eq!(
            span.attributes
                .get("rpc.system")
                .unwrap()
                .as_string()
                .unwrap(),
            "grpc"
        );
        assert_eq!(
            span.attributes
                .get("rpc.service")
                .unwrap()
                .as_string()
                .unwrap(),
            "google.pubsub.v1.Publisher"
        );
        assert_eq!(
            span.attributes
                .get("rpc.method")
                .unwrap()
                .as_string()
                .unwrap(),
            "Publish"
        );
        assert_eq!(
            span.attributes
                .get("server.address")
                .unwrap()
                .as_string()
                .unwrap(),
            "example.com"
        );

        let port = span
            .attributes
            .get("server.port")
            .expect("server.port missing");
        match port {
            google_cloud_test_utils::test_layer::AttributeValue::Int64(val) => {
                assert_eq!(*val, 443)
            }
            google_cloud_test_utils::test_layer::AttributeValue::UInt64(val) => {
                assert_eq!(*val, 443)
            }
            _ => panic!("Unexpected type for server.port: {:?}", port),
        }

        assert_eq!(
            span.attributes
                .get("url.domain")
                .unwrap()
                .as_string()
                .unwrap(),
            "example.com"
        );
        assert_eq!(
            span.attributes
                .get("gcp.client.service")
                .expect("gcp.client.service missing")
                .as_string()
                .unwrap(),
            "test-service"
        );
        assert_eq!(
            span.attributes
                .get("gcp.client.version")
                .expect("gcp.client.version missing")
                .as_string()
                .unwrap(),
            "1.0.0"
        );
        assert_eq!(
            span.attributes
                .get("gcp.client.repo")
                .expect("gcp.client.repo missing")
                .as_string()
                .unwrap(),
            "googleapis/google-cloud-rust"
        );
        assert_eq!(
            span.attributes
                .get("gcp.client.artifact")
                .expect("gcp.client.artifact missing")
                .as_string()
                .unwrap(),
            "test-artifact"
        );
    }

    #[tokio::test]
    async fn test_grpc_span_attributes_without_client_info() {
        let guard = TestLayer::initialize();

        let mut config = ClientConfig::default();
        config.tracing = true;

        let layer = GrpcTowerLayer::new(
            config,
            "example.com".to_string(),
            443,
            "example.com".to_string(),
            None,
        );
        let mut service = layer.layer(DummyService);

        let req = Request::builder()
            .uri(Uri::from_static(
                "https://example.com/google.pubsub.v1.Publisher/Publish",
            ))
            .body(
                Empty::<Bytes>::new()
                    .map_err(|_| tonic::Status::unknown(""))
                    .boxed_unsync(),
            )
            .unwrap();

        let _ = service.call(req).await;

        let spans = TestLayer::capture(&guard);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];

        assert_eq!(span.name, "grpc.request");
        assert_eq!(
            span.attributes
                .get("rpc.system")
                .unwrap()
                .as_string()
                .unwrap(),
            "grpc"
        );
        assert!(span.attributes.get("gcp.client.service").is_none());
        assert!(span.attributes.get("gcp.client.version").is_none());
        assert!(span.attributes.get("gcp.client.repo").is_none());
        assert!(span.attributes.get("gcp.client.artifact").is_none());
    }

    #[tokio::test]
    async fn test_grpc_span_status_code() {
        let guard = TestLayer::initialize();

        let mut config = ClientConfig::default();
        config.tracing = true;

        let layer = GrpcTowerLayer::new(
            config,
            "example.com".to_string(),
            443,
            "example.com".to_string(),
            None,
        );
        let mut service = layer.layer(DummyService);

        let req = Request::builder()
            .uri(Uri::from_static(
                "https://example.com/google.pubsub.v1.Publisher/Publish",
            ))
            .body(
                Empty::<Bytes>::new()
                    .map_err(|_| tonic::Status::unknown(""))
                    .boxed_unsync(),
            )
            .unwrap();

        let _ = service.call(req).await;

        let spans = TestLayer::capture(&guard);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];

        let status = span
            .attributes
            .get("rpc.grpc.status_code")
            .expect("rpc.grpc.status_code missing");
        match status {
            google_cloud_test_utils::test_layer::AttributeValue::Int64(val) => {
                assert_eq!(*val, tonic::Code::Ok as i64)
            }
            _ => panic!("Unexpected type for rpc.grpc.status_code: {:?}", status),
        }
        assert_eq!(
            span.attributes
                .get("otel.status_code")
                .unwrap()
                .as_string()
                .unwrap(),
            "OK"
        );
    }
}
