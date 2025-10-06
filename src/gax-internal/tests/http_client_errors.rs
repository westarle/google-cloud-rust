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

#[cfg(all(test, feature = "_internal-http-client"))]
mod tests {
    use gax::options::*;
    use google_cloud_test_utils::test_layer::TestLayer;
    use opentelemetry_semantic_conventions::trace as semconv;
    use serde_json::json;
    use test_case::test_case;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn test_credentials() -> auth::credentials::Credentials {
        auth::credentials::anonymous::Builder::new().build()
    }

    #[test_case(false; "tracing off")]
    #[test_case(true; "tracing on")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_error_with_status(enable_tracing: bool) -> Result<()> {
        if enable_tracing && !cfg!(feature = "_unstable-o12y") {
            // Skip this test if the feature is disabled.
            return Ok(());
        }

        use serde_json::Value;
        let (endpoint, _server) = echo_server::start().await?;
        let guard = TestLayer::initialize();

        let client_builder = echo_server::builder(endpoint).with_credentials(test_credentials());
        let client_builder = if enable_tracing {
            client_builder.with_tracing()
        } else {
            client_builder
        };
        let client = client_builder.build().await?;
        let builder = client.builder(reqwest::Method::GET, "/error".into());
        let body = json!({});
        let response = client
            .execute::<Value, Value>(builder, Some(body), RequestOptions::default())
            .await;

        match response {
            Ok(v) => panic!("expected an error got={v:?}"),
            Err(e) => {
                assert!(e.http_headers().is_some(), "missing headers in {e:?}");
                let headers = e.http_headers().unwrap();
                assert!(!headers.is_empty(), "empty headers in {e:?}");
                let got = e.status();
                let want = echo_server::make_status()?;
                assert_eq!(got, Some(&want));

                let spans = TestLayer::capture(&guard);
                if enable_tracing {
                    assert_eq!(spans.len(), 1, "Expected 1 span");
                    let span = &spans[0];
                    assert_eq!(span.name, "http_request");
                    let expected_status_code = http::StatusCode::BAD_REQUEST.as_u16();
                    assert_eq!(
                        span.attributes.get(semconv::ERROR_TYPE),
                        Some(&expected_status_code.to_string()),
                        "error.type should be the HTTP status code"
                    );
                } else {
                    assert_eq!(spans.len(), 0, "Expected 0 spans when tracing is disabled");
                }
            }
        }

        Ok(())
    }
}
