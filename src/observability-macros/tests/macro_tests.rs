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
        let _span = info.create_span();
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
        let _span = info.create_span();
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
    let span = expect::span().named("BasicSpanInfo")
        .with_fields(
            expect::field("test.string").with_value(&"basic")
            .and(expect::field("test.i64").with_value(&42i64))
            .only()
        );
    let (subscriber, handle) = subscriber::mock()
        .new_span(span)
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = BasicSpanInfo {
            test_string: "basic".to_string(),
            test_i64: 42,
        };
        let _span = info.create_span();
    });

    handle.assert_finished();
}

#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct InitialOptionsSpanInfo {
    #[observability(key = "opt.string.some")]
    opt_string_some: Option<String>,
    #[observability(key = "opt.string.none")]
    opt_string_none: Option<String>,
    #[observability(key = "opt.i64.some")]
    opt_i64_some: Option<i64>,
    #[observability(key = "opt.i64.none")]
    opt_i64_none: Option<i64>,
}

#[test]
fn test_initial_options_attributes() {
    let span = expect::span().named("InitialOptionsSpanInfo")
        .with_fields(
            expect::field("opt.string.some").with_value(&"hello")
            .and(expect::field("opt.i64.some").with_value(&100i64))
            .only()
        );
    let (subscriber, handle) = subscriber::mock()
        .new_span(span)
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = InitialOptionsSpanInfo {
            opt_string_some: Some("hello".to_string()),
            opt_string_none: None,
            opt_i64_some: Some(100),
            opt_i64_none: None,
        };
        let _span = info.create_span();
    });

    handle.assert_finished();
}

// Corresponds to tests/trybuild/success/options.rs
#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct ResponseOptionsSpanInfo {
    #[observability(key = "test.string.req")]
    test_string_req: String,
    #[observability(key = "test.string.opt", phase = "response")]
    test_string_opt: Option<String>,
    #[observability(key = "test.i64.opt", phase = "response")]
    test_i64_opt: Option<i64>,
}

#[test]
fn test_response_options_struct_attributes() {
    let span = expect::span().named("ResponseOptionsSpanInfo")
        .with_fields(
            expect::field("test.string.req").with_value(&"required")
            // Response phase fields should not be present here
            .only()
        );
    let (subscriber, handle) = subscriber::mock()
        .new_span(span)
        .only()
        .run_with_handle();

    with_default(subscriber, || {
        let info = ResponseOptionsSpanInfo {
            test_string_req: "required".to_string(),
            test_string_opt: Some("optional".to_string()),
            test_i64_opt: Some(200),
        };
        let _span = info.create_span();
        // TODO(#3239): Add test for record_response_attributes, may require custom visitor or different test strategy
    });

    handle.assert_finished();
}
