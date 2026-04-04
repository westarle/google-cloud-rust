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

#[cfg(google_cloud_unstable_tracing)]
mod http_tracing {
    use google_cloud_test_utils::errors::anydump;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn f1_1_http_disablement() -> anyhow::Result<()> {
        integration_tests_o11y::http_tracing::f1_1_http_disablement()
            .await
            .inspect_err(anydump)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn f1_2_f2_2_http_success_case() -> anyhow::Result<()> {
        integration_tests_o11y::http_tracing::f1_2_f2_2_http_success_case()
            .await
            .inspect_err(anydump)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn f1_3_f2_3_f3_2_f3_4_http_server_error() -> anyhow::Result<()> {
        integration_tests_o11y::http_tracing::f1_3_f2_3_f3_2_f3_4_http_server_error()
            .await
            .inspect_err(anydump)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn success_testlayer() -> anyhow::Result<()> {
        integration_tests_o11y::http_tracing::success_testlayer()
            .await
            .inspect_err(anydump)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parse_error() -> anyhow::Result<()> {
        integration_tests_o11y::http_tracing::parse_error()
            .await
            .inspect_err(anydump)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn api_error() -> anyhow::Result<()> {
        integration_tests_o11y::http_tracing::api_error()
            .await
            .inspect_err(anydump)
    }
}
