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
    use gax::retry_policy::{Aip194Strict, NeverRetry, RetryPolicyExt};
    use google_cloud_gax_internal::grpc;
    use google_cloud_gax_internal::options::InstrumentationClientInfo;
    use google_cloud_test_utils::test_layer::{AttributeValue, TestLayer};
    use grpc_server::{google, start_echo_server, start_fixed_responses};

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

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_retry_span() -> anyhow::Result<()> {
        let guard = TestLayer::initialize();
        let (endpoint, _server) = start_fixed_responses(vec![
            Err(tonic::Status::unavailable("try again")),
            Ok(tonic::Response::new(google::test::v1::EchoResponse {
                message: "success".into(),
                ..Default::default()
            })),
        ])
        .await?;

        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());

        let client = grpc::Client::new_with_instrumentation(
            config,
            &endpoint,
            &TEST_INSTRUMENTATION_INFO,
        )
        .await?;

        // Configure retry policy
        let mut request_options = RequestOptions::default();
        request_options.set_retry_policy(Aip194Strict.with_attempt_limit(3));
        request_options.set_idempotency(true);

        // Send request (default retry policy should handle Unavailable)
        let response = send_request_with_options(client, "test", request_options).await?;
        assert_eq!(response.message, "success");

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        
        // Should have 2 spans: one for failure, one for success
        assert_eq!(grpc_spans.len(), 2, "Should capture two grpc.request spans");
        
        let fail_span = grpc_spans[0];
        assert_eq!(fail_span.attributes.get("otel.status_code"), Some(&AttributeValue::String("ERROR".into())));
        
        let success_span = grpc_spans[1];
        assert_eq!(success_span.attributes.get("otel.status_code"), Some(&AttributeValue::String("OK".into())));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_timeout_span() -> anyhow::Result<()> {
        let (endpoint, _server) = start_echo_server().await?;
        let guard = TestLayer::initialize();

        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());

        let client = grpc::Client::new_with_instrumentation(
            config,
            &endpoint,
            &TEST_INSTRUMENTATION_INFO,
        )
        .await?;

        let mut request_options = RequestOptions::default();
        request_options.set_attempt_timeout(std::time::Duration::from_millis(100));
        request_options.set_retry_policy(NeverRetry);

        // Send request with delay > timeout
        let result = send_request_with_delay(client, "test", 200, request_options).await;
        assert!(result.is_err());

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        assert_eq!(grpc_spans.len(), 1);
        
        let span = grpc_spans[0];
        assert_eq!(span.attributes.get("otel.status_code"), Some(&AttributeValue::String("ERROR".into())));
        
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_grpc_cancellation_span() -> anyhow::Result<()> {
        let (endpoint, _server) = start_echo_server().await?;
        let guard = TestLayer::initialize();

        let mut config = google_cloud_gax_internal::options::ClientConfig::default();
        config.tracing = true;
        config.cred = Some(test_credentials());

        let client = grpc::Client::new_with_instrumentation(
            config,
            &endpoint,
            &TEST_INSTRUMENTATION_INFO,
        )
        .await?;

        let mut request_options = RequestOptions::default();
        request_options.set_retry_policy(NeverRetry);

        // Send request with long delay
        let future = send_request_with_delay(client, "test", 5000, request_options);
        
        // Drop the future after a short time
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), future).await;

        let spans = TestLayer::capture(&guard);
        let grpc_spans: Vec<_> = spans.iter().filter(|s| s.name == "grpc.request").collect();
        assert_eq!(grpc_spans.len(), 1);
        
        let span = grpc_spans[0];
        assert_eq!(span.attributes.get("otel.status_code"), Some(&AttributeValue::String("ERROR".into())));
        
        Ok(())
    }

    async fn send_request(
        client: grpc::Client,
        msg: &str,
    ) -> gax::Result<google::test::v1::EchoResponse> {
        send_request_with_options(client, msg, RequestOptions::default()).await
    }

    async fn send_request_with_options(
        client: grpc::Client,
        msg: &str,
        options: RequestOptions,
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
                options,
                "test-only-api-client/1.0",
                "name=test-only",
            )
            .await
            .map(tonic::Response::into_inner)
    }

    async fn send_request_with_delay(
        client: grpc::Client,
        msg: &str,
        delay_ms: u64,
        options: RequestOptions,
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
            delay_ms: Some(delay_ms),
            ..Default::default()
        };
        client
            .execute(
                extensions,
                http::uri::PathAndQuery::from_static("/google.test.v1.EchoService/Echo"),
                request,
                options,
                "test-only-api-client/1.0",
                "name=test-only",
            )
            .await
            .map(tonic::Response::into_inner)
    }
}