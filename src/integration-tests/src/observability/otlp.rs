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

use super::auth::GcpInterceptor;
use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::trace::{SdkTracerProvider, TraceError};
use tonic::transport::ClientTlsConfig;

const GCP_OTLP_ENDPOINT: &str = "https://telemetry.googleapis.com:443";

pub fn init_tracer_provider(
    project_id: &str,
    interceptor: GcpInterceptor,
) -> Result<SdkTracerProvider, TraceError> {
    init_otlp_tracer_provider(project_id, interceptor, GCP_OTLP_ENDPOINT, true)
}

pub(crate) fn init_otlp_tracer_provider(
    _project_id: &str,
    interceptor: GcpInterceptor,
    endpoint: &str,
    use_tls: bool,
) -> Result<SdkTracerProvider, TraceError> {
    // TODO: Add GCP resource attributes once we figure out Resource::new in 0.31

    let mut exporter_builder = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_interceptor(interceptor);

    if use_tls {
        let tls_config = ClientTlsConfig::new().domain_name("telemetry.googleapis.com");
        exporter_builder = exporter_builder.with_tls_config(tls_config);
    }

    let exporter = exporter_builder
        .build()
        .map_err(|e| TraceError::Other(e.into()))?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::auth::GcpInterceptor;
    use tokio::sync::watch;

    #[tokio::test]
    async fn test_init_tracer_provider_no_panic() {
        let (_tx, rx) = watch::channel(None);
        let interceptor = GcpInterceptor::from_rx(rx);

        let result =
            init_otlp_tracer_provider("test-project", interceptor, "http://localhost:12345", false);
        assert!(result.is_ok());
    }
}
