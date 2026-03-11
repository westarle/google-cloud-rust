# Trace Propagation Handover: Google Cloud Rust

This document serves as a handover guide for the End-to-End Trace Context Propagation project. It outlines the current state of our prototyping efforts, the mechanics of our E2E testing, and the critical differences between the current prototype and the expected production-ready implementation.

## 1. Project Overview & Current State

The goal is to implement App Centric Observability by automatically propagating W3C trace context headers (`traceparent`, `tracestate`) in all outgoing HTTP and gRPC requests from the Rust client libraries.

We have structured the work into a stack of four sequential PR branches on GitHub (`westarle/google-cloud-rust`):

*   **PR 1 ([`trace-propagation-deps`](https://github.com/westarle/google-cloud-rust/tree/trace-propagation-deps)):** Introduces the `opentelemetry` dependencies to `gax-internal` and adds a `HeaderInjector` utility in `src/gax-internal/src/observability/propagation.rs` to bridge OpenTelemetry and `http::HeaderMap`.
*   **PR 2 ([`trace-propagation-http`](https://github.com/westarle/google-cloud-rust/tree/trace-propagation-http)):** Wires up context extraction and header injection in `ReqwestClient::request`.
*   **PR 3 ([`trace-propagation-grpc`](https://github.com/westarle/google-cloud-rust/tree/trace-propagation-grpc)):** Wires up context extraction and header injection in the Tonic Tower interceptors (`TracingTowerService` and `NoTracingTowerService`).
*   **PR 4 ([`trace-propagation-e2e`](https://github.com/westarle/google-cloud-rust/tree/trace-propagation-e2e)):** The definitive Spanner integration test (`spanner_tracing.rs`) that proves trace context successfully reaches the Cloud Trace backend and correlates with server-side spans.

**Status:** The core viability is proven. The E2E test successfully passes by injecting a client Trace ID and retrieving the attached server-side Spanner execution spans (`Spanner.CreateSession`, `Spanner.BeginTransaction`).

## 2. Prototype vs. Expected Implementation (Action Items)

While the prototype proves the concept works flawlessly with Google Cloud backends, there are several "hacks" and shortcuts that must be refactored before these PRs can be merged.

### A. The Spanner E2E Tracing Header Hack
**Current Prototype:** In PR 3 (`grpc_tracing.rs`), we unconditionally hardcoded the insertion of `x-goog-spanner-end-to-end-tracing: true` into every gRPC request.
**Expected Implementation:** 
*   This header should *not* be hardcoded globally in `gax-internal`, as it bleeds into other services (like Storage or Pub/Sub).
*   It should be implemented specifically for Spanner.
*   Like the Go client, it should only be injected if the user explicitly opts in via `ClientConfig` OR if the `SPANNER_ENABLE_END_TO_END_TRACING=true` environment variable is present.

### B. Localized Propagator vs. Global State
**Current Prototype:** The `inject_context` helper in PR 1 uses `opentelemetry_sdk::propagation::TraceContextPropagator::new()`. However, `opentelemetry_sdk` is currently only listed under `[dev-dependencies]` in `gax-internal/Cargo.toml`.
**Expected Implementation:**
*   We deliberately chose *not* to rely on the user-configured global `TextMapPropagator` to ensure that Google Cloud APIs *always* receive the correct W3C format, even if the user configures B3 for their internal services.
*   **Action Required:** `opentelemetry_sdk` needs to be added as an optional dependency under `[dependencies]` in `gax-internal/Cargo.toml` and tied to the `_internal-common` feature so the `TraceContextPropagator` can be compiled into the production binary.

### C. Context Extraction Fallback
**Current Prototype:** We currently only extract the context from the `tracing` ecosystem using `tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current())`.
**Expected Implementation:**
*   To support users who use `opentelemetry` directly without the `tracing` bridge, the `inject_context` helper needs a fallback mechanism.
*   **Action Required:** If the `SpanContext` extracted from `tracing` is invalid (i.e., `.is_valid() == false`), the code should fall back to extracting the context from the global ambient state via `opentelemetry::Context::current()`.

### D. Refactoring HTTP and gRPC PRs
**Action Required:** PRs 2 and 3 currently contain duplicate inline logic for global propagator extraction. They need to be rebased on PR 1 and refactored to simply call the unified `crate::observability::propagation::inject_context(...)` helper function.

## 3. End-to-End Testing Guide

The E2E test (`tests/o11y/src/spanner_tracing.rs`) executes real queries against a Spanner instance and polls the Cloud Trace API to verify the backend ingested the traces.

### Prerequisites & Setup
To run the test locally (or to configure the CI environment), you must provision a Google Cloud Project with the correct APIs, a test Spanner instance, and a service account with the proper permissions.

Run the following generic setup commands (replace `YOUR_PROJECT_ID` with your actual project):

```bash
export PROJECT_ID="YOUR_PROJECT_ID"
export SA_NAME="e2e-tracing-test"
export SA_EMAIL="${SA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"

# 1. Enable Required APIs
gcloud services enable spanner.googleapis.com cloudtrace.googleapis.com telemetry.googleapis.com --project=${PROJECT_ID}

# 2. Create a Test Spanner Instance and Database
gcloud spanner instances create trace-propagation-test-instance \
    --config=regional-us-central1 --description="Test Instance" --nodes=1 --project=${PROJECT_ID}
gcloud spanner databases create test-database \
    --instance=trace-propagation-test-instance --project=${PROJECT_ID}

# 3. Create the Service Account
gcloud iam service-accounts create ${SA_NAME} \
    --display-name="E2E Tracing Test Service Account" --project=${PROJECT_ID}

# 4. Grant Required Roles to the Service Account
gcloud projects add-iam-policy-binding ${PROJECT_ID} \
    --member="serviceAccount:${SA_EMAIL}" --role="roles/spanner.databaseUser"
gcloud projects add-iam-policy-binding ${PROJECT_ID} \
    --member="serviceAccount:${SA_EMAIL}" --role="roles/cloudtrace.agent"
gcloud projects add-iam-policy-binding ${PROJECT_ID} \
    --member="serviceAccount:${SA_EMAIL}" --role="roles/cloudtrace.user"

# 5. Grant yourself permission to impersonate the service account locally
gcloud iam service-accounts add-iam-policy-binding ${SA_EMAIL} \
    --member="user:$(gcloud config get-value account)" --role="roles/iam.serviceAccountTokenCreator" --project=${PROJECT_ID}
```

### Running the Test
Once the project is configured, impersonate the service account via Application Default Credentials (ADC) to run the test:

```bash
export PROJECT_ID="YOUR_PROJECT_ID"
export SA_EMAIL="e2e-tracing-test@${PROJECT_ID}.iam.gserviceaccount.com"

# Authenticate locally
gcloud auth application-default login --impersonate-service-account=${SA_EMAIL}

# Run the test
GOOGLE_CLOUD_PROJECT=${PROJECT_ID} RUSTFLAGS="--cfg google_cloud_unstable_tracing" cargo test -p integration-tests-o11y --features "run-integration-tests" --test spanner_tracing -- --nocapture
```

### Understanding Test Success
The test verifies success by asserting that the Trace API returns a trace topology containing:
1.  Our client-generated root span (`e2e-spanner-test`).
2.  The server-side execution spans emitted by the Spanner backend (`Spanner.CreateSession`, `Spanner.BeginTransaction`).
*Note: The test has a deliberate polling backoff (up to ~5 minutes) because backend server spans emitted by `spanner_api_frontend` can take a few minutes to become visible in the Cloud Trace API after the RPC completes.*