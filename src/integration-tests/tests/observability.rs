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
    use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
    use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tonic::{
        metadata::{Ascii, MetadataValue},
        service::Interceptor,
        Status,
    };
    use tracing::Instrument;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::{EnvFilter, Registry};

    #[derive(Clone)]
    struct AuthInterceptor {
        auth_header: MetadataValue<Ascii>,
        project_header: MetadataValue<Ascii>,
    }

    impl Interceptor for AuthInterceptor {
        fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
            request
                .metadata_mut()
                .insert("authorization", self.auth_header.clone());
            request
                .metadata_mut()
                .insert("x-goog-user-project", self.project_header.clone());

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

        let headers_map = match headers {
            auth::credentials::CacheableResource::New { data, .. } => data,
            _ => return Err(anyhow::anyhow!("failed to get new headers")),
        };

        let auth_val = headers_map
            .get(http::header::AUTHORIZATION)
            .ok_or_else(|| anyhow::anyhow!("no authorization header"))?;

        let auth_header = MetadataValue::try_from(auth_val.as_bytes())
            .map_err(|e| anyhow::anyhow!("invalid auth header bytes: {:?}", e))?;

        let project_header = MetadataValue::try_from(&project_id)
            .map_err(|e| anyhow::anyhow!("failed to create project metadata: {:?}", e))?;

        let interceptor = AuthInterceptor {
            auth_header,
            project_header,
        };

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint("https://telemetry.googleapis.com:443")
            .with_tls_config(
                tonic::transport::ClientTlsConfig::new()
                    .with_enabled_roots()
                    .domain_name("telemetry.googleapis.com"),
            )
            .with_interceptor(interceptor)
            .build()?;

        // 3. Configure Tracer Provider
        let resource = opentelemetry_sdk::Resource::builder_empty()
            .with_attributes(vec![
                opentelemetry::KeyValue::new("gcp.project_id", project_id.clone()),
                opentelemetry::KeyValue::new("service.name", "e2e-test"),
            ])
            .build();

        let tracer_provider = SdkTracerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(exporter)
            .build();

        // 4. Install Global Subscriber
        let tracer = tracer_provider.tracer("e2e-test");
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"));

        let subscriber = Registry::default()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(telemetry);

        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global subscriber");

        // 5. Generate Spans
        let root_span = tracing::info_span!("e2e_test_root");
        let trace_id_hex = async {
            let trace_id = tracing::Span::current()
                .context()
                .span()
                .span_context()
                .trace_id()
                .to_string();
            tracing::info!("Generated Trace ID: {}", trace_id);

            tracing::info!("starting e2e test");
            // Make a real API call to generate library spans
            let client = sm::client::SecretManagerService::builder()
                .with_tracing()
                .build()
                .await
                .map_err(|e| anyhow::anyhow!("failed to create client: {:?}", e))?;

            let _ = client
                .list_secrets()
                .set_parent(format!("projects/{}", project_id))
                .send()
                .await;
            tracing::info!("finished e2e test");
            Ok::<String, anyhow::Error>(trace_id)
        }
        .instrument(root_span)
        .await?;

        // 6. Flush
        let _ = tracer_provider.force_flush();

        // 7. Verify Trace via raw HTTP (since v2 client doesn't support read)
        tracing::info!(
            "Verifying trace {} in Cloud Trace via HTTP...",
            trace_id_hex
        );
        let http_client = reqwest::Client::new();
        let url = format!(
            "https://cloudtrace.googleapis.com/v1/projects/{}/traces/{}",
            project_id, trace_id_hex
        );

        let mut found = false;
        for i in 1..=5 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            tracing::info!("Polling attempt {}/5 for {}", i, url);

            let response = http_client
                .get(&url)
                .header(reqwest::header::AUTHORIZATION, auth_val.clone())
                .send()
                .await?;

            if response.status().is_success() {
                tracing::info!("Successfully found trace {} in Cloud Trace!", trace_id_hex);
                found = true;
                break;
            } else {
                tracing::debug!("Trace not found yet: status {}", response.status());
            }
        }

        if !found {
            return Err(anyhow::anyhow!(
                "Timed out waiting for trace {} to appear in Cloud Trace",
                trace_id_hex
            ));
        }

        Ok(())
    }
}