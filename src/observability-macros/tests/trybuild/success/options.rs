use google_cloud_observability_macros::ObservabilityAttributes;

#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct TestSpanInfoWithOptions {
    #[observability(key = "test.string.opt", phase = "response")]
    test_string_opt: Option<String>,

    #[observability(key = "test.i64.opt", phase = "response")]
    test_i64_opt: Option<i64>,
}

fn main() {}
