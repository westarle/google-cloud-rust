// Copyright 2026 Google LLC
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

use crate::Anonymous;
use crate::mock_collector::MockCollector;
use crate::otlp::trace::Builder as TracerProviderBuilder;
use crate::otlp::metrics::Builder as MeterProviderBuilder;
use crate::otlp::logs::Builder as LoggerProviderBuilder;
use google_cloud_storage::client::Storage;
use storage_grpc_mock::google::storage::v2::BidiReadObjectResponse;
use storage_grpc_mock::{MockStorage, start};
use tonic::{Response as TonicResponse, Result as TonicResult, Status, Code};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::test(flavor = "multi_thread")]
pub async fn grpc_failure() -> anyhow::Result<()> {
    let mock_collector = MockCollector::default();
    let otlp_endpoint = mock_collector.start().await;

    let provider = TracerProviderBuilder::new("test-project", "integration-tests")
        .with_endpoint(otlp_endpoint.clone())
        .with_credentials(Anonymous::new().build())
        .build()
        .await?;

    let meter_provider = MeterProviderBuilder::new("test-project", "integration-tests")
        .with_endpoint(otlp_endpoint.parse::<http::Uri>().expect("Failed to parse URI"))
        .with_credentials(Anonymous::new().build())
        .build()
        .await?;
    opentelemetry::global::set_meter_provider(meter_provider.clone());

    let logger_provider = LoggerProviderBuilder::new("test-project", "integration-tests")
        .with_endpoint(otlp_endpoint.parse::<http::Uri>().expect("Failed to parse URI"))
        .with_credentials(Anonymous::new().build())
        .build()
        .await?;

    let _guard = tracing_subscriber::Registry::default()
        .with(crate::tracing::trace_layer(provider.clone()))
        .with(crate::tracing::log_layer(logger_provider.clone()))
        .set_default();

    // 1. Setup Mock gRPC Storage Server to fail immediately
    let (tx, rx) = tokio::sync::mpsc::channel::<TonicResult<BidiReadObjectResponse>>(1);
    tx.send(Err(Status::new(
        Code::NotFound,
        "Object not found",
    )))
    .await?;

    let mut mock = MockStorage::new();
    mock.expect_bidi_read_object()
        .return_once(|_| Ok(TonicResponse::from(rx)));

    let (endpoint, _server) = start("0.0.0.0:0", mock).await?;
    let endpoint = endpoint.trim_end_matches('/');

    let client = Storage::builder()
        .with_endpoint(endpoint)
        .with_credentials(Anonymous::new().build())
        .with_tracing()
        .build()
        .await?;

    // 2. Execute gRPC Request which will fail
    let _ = client.open_object("projects/_/buckets/test-bucket", "test-object").send().await;

    // 3. Flush Spans, Metrics and Logs
    let _ = provider.force_flush();
    let _ = meter_provider.force_flush();
    let _ = logger_provider.force_flush();

    // 4. Verify Spans
    let (_, _, request) = mock_collector
        .traces
        .lock()
        .expect("never poisoned")
        .pop()
        .expect("should have received at least one trace request")
        .into_parts();

    let mut all_spans = Vec::new();
    for rs in request.resource_spans {
        if let Some(resource) = &rs.resource {
            println!("TRACE RESOURCE ATTRIBUTES: {:?}", resource.attributes.iter().map(|kv| kv.key.clone()).collect::<Vec<_>>());
        }
        for ss in rs.scope_spans {
            if let Some(scope) = &ss.scope {
                println!("TRACE SCOPE ATTRIBUTES: {:?}", scope.attributes.iter().map(|kv| kv.key.clone()).collect::<Vec<_>>());
            }
            all_spans.extend(ss.spans);
        }
    }

    let client_span = all_spans
        .iter()
        .find(|s| s.name == "google.storage.v2.Storage/BidiReadObject")
        .expect("Should have a BidiReadObject span");

    assert_eq!(client_span.kind, 3); // SPAN_KIND_CLIENT
    
    // Status Code 2 means ERROR in OTLP
    assert_eq!(client_span.status.as_ref().unwrap().code, 2); 

    let attributes: std::collections::HashMap<String, _> = client_span
        .attributes
        .iter()
        .map(|kv| (kv.key.clone(), kv.value.clone().unwrap()))
        .collect();

    let get_string = |key: &str| -> Option<String> {
        attributes.get(key).and_then(|v| match &v.value {
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => {
                Some(s.clone())
            }
            _ => None,
        })
    };
    
    let get_int = |key: &str| -> Option<i64> {
        attributes.get(key).and_then(|v| match &v.value {
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i)) => {
                Some(*i)
            }
            _ => None,
        })
    };

    println!("ATTRIBUTES = {:?}", attributes.keys());

    assert_eq!(get_string("rpc.system.name").as_deref(), Some("grpc"));
    assert_eq!(get_string("rpc.method").as_deref(), Some("google.storage.v2.Storage/BidiReadObject"));
    // TODO: PRD specifies "rpc.response.status_code" but OTel gRPC emits "rpc.grpc.status_code"
    assert_eq!(get_int("rpc.grpc.status_code"), Some(5)); // NotFound == 5
    assert_eq!(get_string("error.type").as_deref(), Some("NOT_FOUND"));

    // TODO: gRPC GAPIC spans are currently missing the gcp.client.* attributes:
    // assert_eq!(get_string("gcp.client.repo").as_deref(), Some("googleapis/google-cloud-rust"));
    // assert_eq!(get_string("gcp.client.artifact").as_deref(), Some("google-cloud-storage"));
    // assert!(get_string("gcp.client.version").is_some());
    // assert_eq!(get_string("gcp.client.service").as_deref(), Some("storage"));

    // TODO: assert!(get_string("gcp.resource.destination.id").is_some());
    
    let actual_addr = get_string("server.address").unwrap();
    assert!(actual_addr == "127.0.0.1" || actual_addr == "::1" || actual_addr == "0.0.0.0", "address was {}", actual_addr);
    assert!(get_int("server.port").is_some());

    // 5. Verify Metrics
    let mut metrics_requests = mock_collector.metrics.lock().expect("never poisoned");
    let mut found_duration_metric = false;
    while let Some(req) = metrics_requests.pop() {
        let (_, _, metrics_request) = req.into_parts();
        for rm in metrics_request.resource_metrics {
            for sm in rm.scope_metrics {
                if let Some(scope) = &sm.scope {
                    let mut scope_attrs = std::collections::HashMap::new();
                    for kv in &scope.attributes {
                        scope_attrs.insert(kv.key.clone(), kv.value.clone().unwrap());
                    }
                    let get_scope_string = |key: &str| -> Option<String> {
                        scope_attrs.get(key).and_then(|v| match &v.value {
                            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => Some(s.clone()),
                            _ => None,
                        })
                    };
                    assert_eq!(get_scope_string("gcp.client.repo").as_deref(), Some("googleapis/google-cloud-rust"));
                    assert_eq!(get_scope_string("gcp.client.artifact").as_deref(), Some("google-cloud-storage"));
                    assert!(get_scope_string("gcp.client.version").is_some());
                    assert_eq!(get_scope_string("gcp.client.service").as_deref(), Some("storage"));
                }
                for m in sm.metrics {
                    if m.name.contains("test.client.duration") || m.name.contains("gcp.client.request.duration") {
                        found_duration_metric = true;
                        if let Some(opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(h)) = m.data {
                            let point = h.data_points.first().expect("should have a data point");
                            assert_eq!(point.explicit_bounds, vec![0.0, 0.0001, 0.0005, 0.0010, 0.005, 0.010, 0.050, 0.100, 0.5, 1.0, 5.0, 10.0, 60.0, 300.0, 900.0, 3600.0]);
                            
                            let mut metric_attributes = std::collections::HashMap::new();
                            for kv in &point.attributes {
                                metric_attributes.insert(kv.key.clone(), kv.value.clone().unwrap());
                            }

                            let get_metric_string = |key: &str| -> Option<String> {
                                metric_attributes.get(key).and_then(|v| match &v.value {
                                    Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => {
                                        Some(s.clone())
                                    }
                                    _ => None,
                                })
                            };
                            
                            let get_metric_int = |key: &str| -> Option<i64> {
                                metric_attributes.get(key).and_then(|v| match &v.value {
                                    Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i)) => {
                                        Some(*i)
                                    }
                                    _ => None,
                                })
                            };

                            assert_eq!(get_metric_string("rpc.system.name").as_deref(), Some("grpc"));
                            assert_eq!(get_metric_string("rpc.method").as_deref(), Some("google.storage.v2.Storage/BidiReadObject"));
                            // TODO: PRD specifies "rpc.response.status_code" but OTel gRPC emits "rpc.grpc.status_code"
                            assert_eq!(get_metric_string("rpc.grpc.status_code").as_deref(), Some("NOT_FOUND")); 
                            assert_eq!(get_metric_string("error.type").as_deref(), Some("NOT_FOUND"));

                            let actual_addr = get_metric_string("server.address").unwrap();
                            assert!(actual_addr == "127.0.0.1" || actual_addr == "::1" || actual_addr == "0.0.0.0", "address was {}", actual_addr);
                            assert!(get_metric_int("server.port").is_some());
                        }
                    }
                }
            }
        }
    }
    assert!(found_duration_metric, "Should have found duration metric");

    // 6. Verify Logs
    let logs_requests = mock_collector.logs.lock().unwrap();
    let log_event = logs_requests
        .iter()
        .flat_map(|r| r.get_ref().resource_logs.clone())
        .flat_map(|rl| rl.scope_logs)
        .filter(|sl| {
            sl.scope
                .as_ref()
                .is_some_and(|i| i.name == "google_cloud_gax_internal::observability::errors")
        })
        .flat_map(|sl| sl.log_records)
        .find(|l| l.span_id == client_span.span_id)
        .unwrap_or_else(|| {
            panic!("cannot find log matching span {:?}", client_span.span_id)
        });

    assert_eq!(log_event.trace_id, client_span.trace_id, "Log traceId correlation failed");
    assert_eq!(log_event.span_id, client_span.span_id, "Log spanId correlation failed");

    let mut got_log_attrs = std::collections::HashMap::new();
    for kv in &log_event.attributes {
        let val_str = match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(s)) => s.clone(),
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(i)) => i.to_string(),
            _ => format!("{:?}", kv.value),
        };
        got_log_attrs.insert(kv.key.clone(), val_str);
    }

    println!("LOG ATTRIBUTES = {:?}", got_log_attrs.keys());

    assert_eq!(got_log_attrs.get("error.type").map(String::as_str), Some("NOT_FOUND"));
    // TODO: assert_eq!(got_log_attrs.get("rpc.grpc.status_code").map(String::as_str), Some("5"));

    // OTel L4 Actionable Error Logger correctly translates gRPC codes to names for the logs
    assert_eq!(got_log_attrs.get("rpc.response.status_code").map(String::as_str), Some("NOT_FOUND"));
    
    // TODO: L4 Actionable Error Logs are currently missing these PRD attributes:
    // assert_eq!(got_log_attrs.get("rpc.system.name").map(String::as_str), Some("grpc"));
    // assert_eq!(got_log_attrs.get("rpc.method").map(String::as_str), Some("google.storage.v2.Storage/BidiReadObject"));
    // assert_eq!(got_log_attrs.get("gcp.client.repo").map(String::as_str), Some("googleapis/google-cloud-rust"));
    // assert_eq!(got_log_attrs.get("gcp.client.language").map(String::as_str), Some("rust"));
    // assert_eq!(got_log_attrs.get("gcp.client.service").map(String::as_str), Some("storage"));
    // assert_eq!(got_log_attrs.get("server.address").map(String::as_str), Some("..."));

    assert_eq!(log_event.severity_text, "DEBUG", "severity_text mismatch");

    Ok(())
}
