use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Input {
    pub(super) calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "route", rename_all = "snake_case")]
pub(super) enum Call {
    List(ListCall),
    Info(InfoCall),
}

#[derive(Deserialize)]
pub(super) struct ListCall {
    pub(super) page: i64,
    pub(super) page_size: i64,
    #[serde(default)]
    pub(super) alias: bool,
    #[serde(flatten)]
    pub(super) setup: Setup,
}

#[derive(Deserialize)]
pub(super) struct InfoCall {
    pub(super) vector_store_id: String,
    #[serde(flatten)]
    pub(super) setup: Setup,
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
