use serde::Deserialize;

pub(crate) type Pair = (String, String);

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Operation {
    Run,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub operation: Operation,
    #[serde(default)]
    pub debug: Vec<DebugScenario>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticScenario>,
    #[serde(default)]
    pub bodies: Vec<BodyScenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebugScenario {
    #[serde(default)]
    pub headers: Vec<Pair>,
    #[serde(default)]
    pub oauth2_headers: Option<Vec<Pair>>,
    #[serde(default)]
    pub litellm_api_key: Option<String>,
    pub auth_resolution: String,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_auth_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticScenario {
    #[serde(default)]
    pub records: Vec<Pair>,
    #[serde(default)]
    pub through_context: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyScenario {
    pub body: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
}
