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

use crate::Result;
use crate::observability::{otlp, tracing as obs_tracing};
use showcase::client::Echo;
use tracing_subscriber::{layer::SubscriberExt, Registry};
use std::env;
use opentelemetry::trace::TraceContextExt;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub async fn run() -> Result<()> {
    // Only run this test if the environment variable is set, otherwise skip it.
    // This prevents failures in environments where we just want to run standard showcase tests.
    let project_id = match env::var("GOOGLE_CLOUD_PROJECT") {
        Ok(pid) => pid,
        Err(_) => {
            tracing::warn!("Skipping observability test: GOOGLE_CLOUD_PROJECT not set");
            return Ok(());
        }
    };
    
    let service_name = "showcase-observability-test";

    // 1. Initialize OTLP Provider
    let provider = otlp::CloudTelemetryTracerProviderBuilder::new(&project_id, service_name)
        .build()
        .await?;

    // 2. Initialize Tracing Subscriber
    let layer = obs_tracing::layer(provider.clone());
    
    // We don't need a fmt layer here as the parent `showcase::run` might have one,
    // or we can rely on the parent's subscriber for logging if we compose them.
    // However, `set_default` replaces the subscriber for the current thread.
    // To keep seeing logs, we should probably include a fmt layer or try to compose with the parent.
    // For simplicity in this test, we'll just use our own subscriber which includes the OTLP layer.
    // We add a fmt layer to ensure we can still see what's happening.
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_test_writer();

    let subscriber = Registry::default()
        .with(layer)
        .with(fmt_layer)
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")));
    
    // Use set_default to scope it to this function
    let _guard = tracing::subscriber::set_default(subscriber);

    // 3. Execute API call within a span
    let root_span = tracing::info_span!("showcase_observability_root_span");
    let _enter = root_span.enter();

    // Capture Trace ID
    let span = tracing::Span::current();
    let context = span.context();
    let trace_id = context.span().span_context().trace_id();
    tracing::info!("Generated Trace ID: {}", trace_id);

    // Connect to local Showcase
    let client = Echo::builder()
        .with_endpoint("http://localhost:7469")
        .with_credentials(auth::credentials::anonymous::Builder::new().build())
        .with_tracing()
        .build()
        .await?;

    tracing::info!("Sending Echo request...");
    let response = client
        .echo()
        .set_content("hello tracing")
        .send()
        .await;
    tracing::info!("Request finished.");

    if let Err(e) = response {
        return Err(anyhow::anyhow!("Failed to echo: {:?}", e).into());
    }

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

    tracing::info!("Polling Cloud Trace API: {}", url);

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
                        .map(|s| s == "showcase_observability_root_span")
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
                        tracing::info!("Found trace with root span and child span: {}", name);
                        return Ok(());
                    }
                }
            }
            tracing::info!("Trace found but incomplete (attempt {}). Retrying...", i);
        } else {
            tracing::info!("Trace not found yet (attempt {}): status {}", i, resp.status());
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    anyhow::bail!("Trace {} not found or incomplete in project {} after polling", trace_id, project_id);
}
