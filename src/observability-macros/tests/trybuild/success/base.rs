use google_cloud_observability_macros::ObservabilityAttributes;


#[derive(Debug, Clone, ObservabilityAttributes)]
pub(crate) struct HttpSpanInfo {
    #[observability(key = "otel.kind")]
    otel_kind: String,

    #[observability(key = "otel.status", phase = "response")]
    otel_status: String,
}

fn main() {}
