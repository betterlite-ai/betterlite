use serde::Deserialize;

// the per-deployment litellm_params key the partner count call reads besides the shared Vertex settings
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct PartnerCountOptions {
    pub vertex_count_tokens_location: Option<String>,
}
