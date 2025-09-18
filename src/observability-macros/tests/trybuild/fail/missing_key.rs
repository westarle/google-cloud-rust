use google_cloud_observability_macros::ObservabilityAttributes;


#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct HttpSpanInfo {
    #[observability(phase = "response")]
    otel_status: String,
}

fn main() {}
