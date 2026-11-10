use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Route {
        cases: Vec<RouteCase>,
    },
    Partner {
        models: Vec<String>,
    },
    Capabilities {
        cases: Vec<CapabilityCase>,
    },
    Lyria {
        cases: Vec<LyriaCase>,
    },
    Labels {
        cases: Vec<LabelCase>,
    },
    Metadata {
        logged_objects: Vec<Json>,
        call_details: Vec<Json>,
    },
    Timestamp {
        zone: String,
        values: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouteCase {
    pub(crate) model: String,
    pub(crate) base_model: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityCase {
    pub(crate) model: String,
    pub(crate) provider: ProviderName,
    pub(crate) system: LookupOutcome,
    pub(crate) schema: LookupOutcome,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderName {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LookupOutcome {
    True,
    False,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LyriaCase {
    pub(crate) model: String,
    pub(crate) runtime: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelCase {
    pub(crate) optional_params: Option<Json>,
    pub(crate) litellm_params: Option<Json>,
}
