#![cfg(google_cloud_unstable_tracing)]

use integration_tests::observability::{otlp, tracing as obs_tracing};
use sm::client::SecretManagerService;
use tracing_subscriber::{layer::SubscriberExt, Registry};
use std::env;
use opentelemetry::trace::TraceContextExt;
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg(feature = "run-integration-tests")]
async fn test_end_to_end_tracing() -> anyhow::Result<()> {
    let project_id = env::var("GOOGLE_CLOUD_PROJECT")
        .expect("GOOGLE_CLOUD_PROJECT must be set");
    let service_name = "e2e-test-service";

    // 1. Initialize OTLP Provider
    let provider = otlp::CloudTelemetryTracerProviderBuilder::new(&project_id, service_name)
        .build()
        .await?;

    // 2. Initialize Tracing Subscriber
    let layer = obs_tracing::layer(provider.clone());
    
    // Add a fmt layer to see logs in stdout
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_test_writer();
    
    let subscriber = Registry::default()
        .with(layer)
        .with(fmt_layer)
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")));
    
    // Use set_default to scope it to this test
    let _guard = tracing::subscriber::set_default(subscriber);

    // 3. Execute API call within a span
    let root_span = tracing::info_span!("e2e_test_root_span");
    let _enter = root_span.enter();

    // Capture Trace ID
    let span = tracing::Span::current();
    let context = span.context();
    let trace_id = context.span().span_context().trace_id();
    println!("Generated Trace ID: {}", trace_id);

    // Use explicit credentials to match previous working test
    let scopes = ["https://www.googleapis.com/auth/cloud-platform"];
    let credentials = auth::credentials::Builder::default()
        .with_scopes(scopes)
        .build()?;

    let client = SecretManagerService::builder()
        .with_credentials(credentials)
        .with_tracing()
        .build()
        .await?;

    println!("Client built, sending request...");
    let response = client
        .list_secrets()
        .set_parent(format!("projects/{}", project_id))
        .set_page_size(1)
        .send()
        .await;
    println!("Request finished.");

    assert!(response.is_ok(), "Failed to list secrets: {:?}", response.err());

    // Drop span to ensure it ends
    drop(_enter);
    drop(root_span);

    // 4. Flush spans
    provider.force_flush()?;

    // 5. Verify Trace
    // We need to wait a bit for ingestion
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    verify_trace_existence(&project_id, &trace_id.to_string()).await?;

    Ok(())
}

async fn verify_trace_existence(project_id: &str, trace_id: &str) -> anyhow::Result<()> {
    // Use ADC for verification request
    let credentials = auth::credentials::Builder::default()
        .with_scopes(vec!["https://www.googleapis.com/auth/cloud-platform"])
        .build()?;
    let headers = credentials.headers(http::Extensions::new()).await?;
    
    let token_header_value = match headers {
        auth::credentials::CacheableResource::New { data, .. } => {
            data.get(http::header::AUTHORIZATION)
                .ok_or_else(|| anyhow::anyhow!("No Authorization header"))?
                .clone()
        }
        _ => anyhow::bail!("Unexpected NotModified"),
    };

    let client = reqwest::Client::new();
    let url = format!(
        "https://cloudtrace.googleapis.com/v2/projects/{}/traces/{}",
        project_id, trace_id
    );

    println!("Polling Cloud Trace API: {}", url);

    // Poll for a while
    for i in 0..20 {
        let resp = client.get(&url)
            .header(http::header::AUTHORIZATION, token_header_value.clone())
            .send()
            .await?;

        if resp.status().is_success() {
            let body = resp.text().await?;
            let trace: serde_json::Value = serde_json::from_str(&body)?;

            if let Some(spans) = trace.get("spans").and_then(|s| s.as_array()) {
                // Find root span
                let root_span = spans.iter().find(|s| {
                    s.get("displayName")
                        .and_then(|n| n.get("value"))
                        .and_then(|v| v.as_str())
                        .map(|s| s == "e2e_test_root_span")
                        .unwrap_or(false)
                });

                if let Some(root) = root_span {
                    let root_id = root.get("spanId").and_then(|s| s.as_str()).unwrap_or("");
                    
                    // Find child span (HTTP span)
                    let child_span = spans.iter().find(|s| {
                        s.get("parentSpanId")
                            .and_then(|p| p.as_str())
                            .map(|p| p == root_id)
                            .unwrap_or(false)
                    });

                    if let Some(child) = child_span {
                        let name = child.get("displayName")
                            .and_then(|n| n.get("value"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        println!("Found trace with root span and child span: {}", name);
                        return Ok(());
                    }
                }
            }
            println!("Trace found but incomplete (attempt {}). Retrying...", i);
        } else {
            println!("Trace not found yet (attempt {}): status {}", i, resp.status());
            if i == 19 {
                 let body = resp.text().await?;
                 println!("Last response body: {}", body);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    anyhow::bail!("Trace {} not found or incomplete in project {} after polling", trace_id, project_id);
}
