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
    use google_cloud_gax_internal::options::InstrumentationClientInfo;
    use google_cloud_test_utils::test_layer::{AttributeValue, TestLayer};
    use grpc_server::{google, start_echo_server};

    const TEST_SERVICE: &str = "test.service";
    const TEST_VERSION: &str = "1.2.3";
    const TEST_ARTIFACT: &str = "google-cloud-test";
    const TEST_HOST: &str = "test.googleapis.com";

    lazy_static::lazy_static! {
        static ref TEST_INSTRUMENTATION_INFO: InstrumentationClientInfo = {
            let mut info = InstrumentationClientInfo::default();
            info.service_name = TEST_SERVICE;
            info.client_version = TEST_VERSION;
            info.client_artifact = TEST_ARTIFACT;
            info.default_host = TEST_HOST;
            info
        };
    }

    fn test_credentials() -> auth::credentials::Credentials {
        auth::credentials::anonymous::Builder::new().build()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_success_span() -> anyhow::Result<()> {
        let (endpoint, _server) = start_echo_server().await?;
        let guard = TestLayer::initialize();

        // Configure client with tracing enabled
        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());

        let client = grpc::Client::new_with_instrumentation(
            config,
            &endpoint,
            &TEST_INSTRUMENTATION_INFO,
        )
        .await?;

        // Send a request
        let _response = send_request(client, "test message").await?;

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        assert_eq!(
            grpc_spans.len(),
            1,
            "Should capture one grpc.request span: {:?}",
            grpc_spans
        );
        let span = grpc_spans[0];
        let attrs = &span.attributes;

        assert_eq!(span.name, "grpc.request");
        assert_eq!(
            attrs.get("rpc.system"),
            Some(&AttributeValue::String("grpc".into()))
        );
        assert_eq!(
            attrs.get("rpc.service"),
            Some(&AttributeValue::String("google.test.v1.EchoService".into()))
        );
        assert_eq!(
            attrs.get("rpc.method"),
            Some(&AttributeValue::String("Echo".into()))
        );
        assert_eq!(
            attrs.get("gcp.client.service"),
            Some(&AttributeValue::String(TEST_SERVICE.into()))
        );
        assert_eq!(
            attrs.get("gcp.client.version"),
            Some(&AttributeValue::String(TEST_VERSION.into()))
        );
        assert_eq!(
            attrs.get("gcp.client.repo"),
            Some(&AttributeValue::String(
                "googleapis/google-cloud-rust".into()
            ))
        );
        assert_eq!(
            attrs.get("gcp.client.artifact"),
            Some(&AttributeValue::String(TEST_ARTIFACT.into()))
        );
        assert_eq!(
            attrs.get("rpc.grpc.status_code"),
            Some(&AttributeValue::Int64(tonic::Code::Ok as i64))
        );
        assert_eq!(
            attrs.get("otel.status_code"),
            Some(&AttributeValue::String("OK".into()))
        );

        // Verify server address and port are present (values depend on dynamic port)
        assert!(attrs.contains_key("server.address"));
        assert!(attrs.contains_key("server.port"));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_error_span() -> anyhow::Result<()> {
        // Start server but we'll send a request that triggers an error if possible,
        // or just connect to an invalid endpoint to trigger connection error.
        // Actually, let's use the echo server and send a request that returns an error status.
        // The echo server might not support forcing an error easily without modifying it.
        // Let's try connecting to a closed port.
        let guard = TestLayer::initialize();

        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());

        // Use a port that is likely closed
        let endpoint = "http://localhost:1";
        let client = grpc::Client::new_with_instrumentation(
            config,
            endpoint,
            &TEST_INSTRUMENTATION_INFO,
        )
        .await?;

        let result = send_request(client, "test message").await;
        assert!(result.is_err());

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        assert!(
            !grpc_spans.is_empty(),
            "Should capture at least one grpc.request span: {:?}",
            grpc_spans
        );
        let span = grpc_spans[0];
        let attrs = &span.attributes;

        assert_eq!(span.name, "grpc.request");
        assert_eq!(
            attrs.get("rpc.system"),
            Some(&AttributeValue::String("grpc".into()))
        );
        assert_eq!(
            attrs.get("otel.status_code"),
            Some(&AttributeValue::String("ERROR".into()))
        );
        // Status code might not be present if it failed before getting a response status
        // But we should have otel.status_code = ERROR

        Ok(())
    }

    async fn send_request(
        client: grpc::Client,
        msg: &str,
    ) -> gax::Result<google::test::v1::EchoResponse> {
        let extensions = {
            let mut e = tonic::Extensions::new();
            e.insert(tonic::GrpcMethod::new(
                "google.test.v1.EchoServices",
                "Echo",
            ));
            e
        };
        let request = google::test::v1::EchoRequest {
            message: msg.into(),
            ..Default::default()
        };
        client
            .execute(
                extensions,
                http::uri::PathAndQuery::from_static("/google.test.v1.EchoService/Echo"),
                request,
                RequestOptions::default(),
                "test-only-api-client/1.0",
                "name=test-only",
            )
            .await
            .map(tonic::Response::into_inner)
    }
}
