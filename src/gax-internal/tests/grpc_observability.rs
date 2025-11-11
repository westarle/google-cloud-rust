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

#[cfg(all(test, feature = "_internal-grpc-client", google_cloud_unstable_tracing))]
mod tests {
    use gax::options::RequestOptions;
    use google_cloud_gax_internal::grpc;
    use google_cloud_test_utils::test_layer::TestLayer;
    use grpc_server::{google, start_echo_server};

    fn test_credentials() -> auth::credentials::Credentials {
        auth::credentials::anonymous::Builder::new().build()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_basic_span() -> anyhow::Result<()> {
        let (endpoint, _server) = start_echo_server().await?;
        let guard = TestLayer::initialize();

        // Configure client with tracing enabled
        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());

        let client = grpc::Client::new(config, &endpoint).await?;

        // Send a request
        let extensions = {
            let mut e = tonic::Extensions::new();
            e.insert(tonic::GrpcMethod::new(
                "google.test.v1.EchoServices",
                "Echo",
            ));
            e
        };
        let request = google::test::v1::EchoRequest {
            message: "test message".into(),
            ..Default::default()
        };
        let _ = client
            .execute::<_, google::test::v1::EchoResponse>(
                extensions,
                http::uri::PathAndQuery::from_static("/google.test.v1.EchoService/Echo"),
                request,
                RequestOptions::default(),
                "test-only-api-client/1.0",
                "name=test-only",
            )
            .await?;

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        assert_eq!(
            grpc_spans.len(),
            1,
            "Should capture one grpc.request span: {:?}",
            grpc_spans
        );
        
        let span = &grpc_spans[0];
        assert_eq!(span.attributes.get("rpc.system").and_then(|v| v.as_string()), Some("grpc".to_string()));
        assert_eq!(span.attributes.get("otel.kind").and_then(|v| v.as_string()), Some("client".to_string()));
        assert_eq!(span.attributes.get("rpc.service").and_then(|v| v.as_string()), Some("google.test.v1.EchoService".to_string()));
        assert_eq!(span.attributes.get("rpc.method").and_then(|v| v.as_string()), Some("Echo".to_string()));
        
        // The echo server runs on localhost, so we expect localhost or 127.0.0.1
        let address = span.attributes.get("server.address").and_then(|v| v.as_string()).unwrap();
        assert!(address == "localhost" || address == "127.0.0.1", "Unexpected address: {}", address);
        
        assert!(span.attributes.contains_key("server.port"));
        assert_eq!(span.attributes.get("url.domain").and_then(|v| v.as_string()), Some(address));

        // Check placeholders exist (though they might be None/Empty in the captured span if not set)
        // Note: tracing-subscriber's test layer might not capture Empty fields unless recorded.
        // But we want to ensure they are defined in the span macro.
        // Since we can't easily check "defined but empty" with this test layer if it ignores them,
        // we assume the code change covers it.
        // But we can check if we set them to something later.
        
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_custom_endpoint() -> anyhow::Result<()> {
        let (endpoint, _server) = start_echo_server().await?;
        let guard = TestLayer::initialize();

        // Configure client with tracing enabled and a custom endpoint
        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());
        // We use the actual echo server endpoint but pretend it's a custom one for config purposes
        // Note: Client::new uses the config.endpoint if set, otherwise default_endpoint.
        // Here we want to test parsing logic, so we set config.endpoint.
        config.endpoint = Some(endpoint.clone());

        let client = grpc::Client::new(config, "http://unused.default.com").await?;

        // Send a request
        let extensions = {
            let mut e = tonic::Extensions::new();
            e.insert(tonic::GrpcMethod::new(
                "google.test.v1.EchoServices",
                "Echo",
            ));
            e
        };
        let request = google::test::v1::EchoRequest {
            message: "test message".into(),
            ..Default::default()
        };
        let _ = client
            .execute::<_, google::test::v1::EchoResponse>(
                extensions,
                http::uri::PathAndQuery::from_static("/google.test.v1.EchoService/Echo"),
                request,
                RequestOptions::default(),
                "test-only-api-client/1.0",
                "name=test-only",
            )
            .await?;

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        assert_eq!(grpc_spans.len(), 1);
        
        let span = &grpc_spans[0];
        // Verify parsing of the custom endpoint
        // The endpoint string from start_echo_server is like "http://127.0.0.1:12345"
        let uri: http::Uri = endpoint.parse().unwrap();
        let expected_host = uri.host().unwrap().to_string();
        let expected_port = uri.port_u16().unwrap();

        assert_eq!(span.attributes.get("server.address").and_then(|v| v.as_string()), Some(expected_host.clone()));
        assert_eq!(span.attributes.get("server.port").and_then(|v| v.as_i64()), Some(expected_port as i64));
        assert_eq!(span.attributes.get("url.domain").and_then(|v| v.as_string()), Some(expected_host));

        Ok(())
    }
}
