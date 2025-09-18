use google_cloud_observability_macros::ObservabilityAttributes;
use tracing::subscriber::with_default;
use tracing_mock::{expect, subscriber};

// --- Test Cases ---

#[derive(ObservabilityAttributes)]
#[observability(name = "test_span")]
pub struct TestInfo {
    #[observability(key = "my.string")]
    s: String,
    #[observability(key = "my.int")]
    i: i64,
}

#[test]
fn test_create_span_name() {
    let span = expect::span().named("test_span");
    let (subscriber, handle) = subscriber::mock()
        .new_span(span.clone())
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = TestInfo { s: "hello".to_string(), i: 123 };
        let span = info.create_span();
    });

    handle.assert_finished();
}

#[derive(ObservabilityAttributes)]
pub struct DefaultNameInfo {
    #[observability(key = "my.field")]
    f: String,
}

#[test]
fn test_create_span_default_name() {
    let span = expect::span().named("DefaultNameInfo");
    let (subscriber, handle) = subscriber::mock()
        .new_span(span.clone())
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = DefaultNameInfo { f: "world".to_string() };
        let span = info.create_span();
    });

    handle.assert_finished();
}

// Corresponds to tests/trybuild/success/base.rs
#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct BasicSpanInfo {
    #[observability(key = "test.string")]
    test_string: String,
    #[observability(key = "test.i64")]
    test_i64: i64,
}

#[test]
fn test_basic_struct_attributes() {
    let span = expect::span().named("BasicSpanInfo");
    let (subscriber, handle) = subscriber::mock()
        .new_span(span.clone())
        .enter(span.clone())
        .exit(span.clone())
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = BasicSpanInfo {
            test_string: "basic".to_string(),
            test_i64: 42,
        };
        let span = info.create_span();
        let _enter = span.enter();
    });

    handle.assert_finished();
    // TODO: Add attribute expectations to the mock builder
}

// Corresponds to tests/trybuild/success/options.rs
#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct OptionsSpanInfo {
    #[observability(key = "test.string.req")]
    test_string_req: String,
    #[observability(key = "test.string.opt", phase = "response")]
    test_string_opt: Option<String>,
    #[observability(key = "test.i64.opt", phase = "response")]
    test_i64_opt: Option<i64>,
}

#[test]
fn test_options_struct_attributes() {
    let span = expect::span().named("OptionsSpanInfo");
    let (subscriber, handle) = subscriber::mock()
        .new_span(span.clone())
        .enter(span.clone())
        .exit(span.clone())
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = OptionsSpanInfo {
            test_string_req: "required".to_string(),
            test_string_opt: Some("optional".to_string()),
            test_i64_opt: None,
        };
        let span = info.create_span();
        span.in_scope(|| {
            // TODO: Call record_response_attributes once macro generates it
        });
    });

    handle.assert_finished();
    // TODO: Add attribute expectations to the mock builder
}