// Copyright 2024 Google LLC
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

#[cfg(all(test, feature = "run-integration-tests"))]
mod observability {
    use auth::credentials::Builder;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tonic::{metadata::MetadataValue, service::Interceptor, Status};
    use tracing::Instrument;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Registry;

    #[derive(Clone)]
    struct AuthInterceptor {
        token: String,
    }

    impl Interceptor for AuthInterceptor {
        fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
            let val = MetadataValue::try_from(&self.token)
                .map_err(|e| Status::internal(format!("failed to create metadata value: {}", e)))?;
            request.metadata_mut().insert("authorization", val);
            Ok(request)
        }
    }

    #[tokio::test]
    async fn test_export_spans_grpc() -> integration_tests::Result<()> {
        // 1. Setup Auth
        let project_id = integration_tests::project_id()?;
        let scopes = ["https://www.googleapis.com/auth/cloud-platform"];
        let creds = Builder::default()
            .with_scopes(scopes)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build credentials: {:?}", e))?;
        let headers = creds
            .headers(http::Extensions::new())
            .await
            .map_err(|e| anyhow::anyhow!("failed to get headers: {:?}", e))?;

        let token = match headers {
            auth::credentials::CacheableResource::New { data, .. } => data
                .get(http::header::AUTHORIZATION)
                .ok_or_else(|| anyhow::anyhow!("no authorization header"))?
                .to_str()
                .map_err(|e| anyhow::anyhow!("invalid authorization header: {:?}", e))?
                .to_string(),
            _ => return Err(anyhow::anyhow!("failed to get new headers")),
        };

        // 2. Configure OTLP gRPC Exporter
        let interceptor = AuthInterceptor {
            token, // It already has "Bearer " prefix
        };

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint("https://telemetry.googleapis.com:443")
            .with_interceptor(interceptor)
            .build()?;

        // 3. Configure Tracer Provider
        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();

        // 4. Install Global Subscriber
        let tracer = tracer_provider.tracer("e2e-test");
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        let subscriber = Registry::default().with(telemetry);

        tracing::subscriber::with_default(subscriber, || async {
            // 5. Generate Spans
            let root_span = tracing::info_span!("e2e_test_root");
            async {
                tracing::info!("starting e2e test");
                // Make a real API call to generate library spans
                let client = sm::client::SecretManagerService::builder()
                    .build()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to create client: {:?}", e))?;

                let _ = client
                    .list_secrets()
                    .set_parent(format!("projects/{}", project_id))
                    .send()
                    .await;
                tracing::info!("finished e2e test");
                Ok::<(), anyhow::Error>(())
            }
            .instrument(root_span)
            .await
        })
        .await?;

        // 6. Flush
        let _ = tracer_provider.force_flush();

        Ok(())
    }
}