use google_cloud_observability_macros::ObservabilityAttributes;

#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct UnsupportedTypeSpanInfo {
    #[observability(key = "test.unsupported")]
    test_unsupported: bool,
}

fn main() {}
