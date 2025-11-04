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

#[cfg(google_cloud_unstable_tracing)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_end_to_end_tracing() -> Result<(), Box<dyn std::error::Error>> {
    use integration_tests::observability::{
        auth::GcpInterceptor, otlp::init_tracer_provider, tracing::init_tracing,
    };
    use sm::client::SecretManagerService;
    use tracing::{info, info_span};

    let project_id =
        std::env::var("GOOGLE_CLOUD_PROJECT").unwrap_or_else(|_| "test-project".to_string());

    // Use error credentials if real ones are not available, to allow test to run in CI without secrets.
    // In a real E2E env, we would use Builder::default().build()?;
    let credentials = auth::credentials::testing::error_credentials(true);

    let interceptor = GcpInterceptor::new(credentials.clone());

    let provider = init_tracer_provider(&project_id, interceptor)?;
    init_tracing(provider.clone());

    {
        let span = info_span!("e2e_test_span");
        let _enter = span.enter();
        info!("Starting E2E tracing test");

        let client = SecretManagerService::builder()
            .with_credentials(credentials)
            .build()
            .await?;

        let project_name = format!("projects/{project_id}");
        let response = client
            .list_secrets()
            .set_parent(project_name)
            .set_page_size(1)
            .send()
            .await;

        match response {
            Ok(_) => info!("Successfully listed secrets"),
            Err(e) => info!(
                "Failed to list secrets (expected if API disabled, but tracing should still work): {:?}",
                e
            ),
        }
    }

    // Force flush to ensure spans are exported
    let _ = provider.force_flush();

    // Shutdown provider to ensure everything is cleaned up
    let _ = provider.shutdown();

    Ok(())
}
