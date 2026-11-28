use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub(crate) struct Input {
    pub(super) calls: Vec<Call>,
}

#[derive(Deserialize)]
pub(super) struct Call {
    pub(super) route: Route,
    pub(super) body: Value,
    #[serde(flatten)]
    pub(super) setup: Setup,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(super) enum Route {
    New,
    Delete,
    Update,
}

#[derive(Deserialize)]
pub(super) struct Setup {
    #[serde(default)]
    pub(super) database: Option<Vec<LiteLlmManagedVectorStore>>,
    #[serde(default)]
    pub(super) registry: Option<Vec<LiteLlmManagedVectorStore>>,
    pub(super) caller: Caller,
    #[serde(default)]
    pub(super) disabled: bool,
}

#[derive(Deserialize)]
pub(super) struct Caller {
    pub(super) user_id: String,
    pub(super) user_role: String,
    #[serde(default)]
    pub(super) team_id: Option<String>,
}
