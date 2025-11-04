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

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::{Registry, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing(tracer_provider: SdkTracerProvider) {
    let tracer = tracer_provider.tracer("google-cloud-rust-integration-test");

    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    // Ignore error if subscriber is already set
    let _ = Registry::default()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(telemetry)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::trace::InMemorySpanExporter;
    use tracing::{Level, info, span};

    #[test]
    fn test_tracing_integration() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();

        let tracer = provider.tracer("test");
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        let subscriber = Registry::default().with(telemetry);

        tracing::subscriber::with_default(subscriber, || {
            let span = span!(Level::INFO, "test_span");
            let _enter = span.enter();
            info!("test event");
        });

        // Force flush to ensure spans are exported
        let _ = provider.force_flush();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "test_span");
    }
}
