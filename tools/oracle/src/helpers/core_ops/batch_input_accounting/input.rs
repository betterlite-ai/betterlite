use betterlite_core::api::batch_input_accounting::input::BatchInputProvider;
use betterlite_core::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use betterlite_core::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_with::{As, DefaultOnNull};

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    #[default]
    Allow,
    Deny,
    Fail,
    Http,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Lookup {
    #[default]
    Loaded,
    Failed,
    Http,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Hook {
    #[default]
    Endpoints,
    Missing,
    Plain,
}

// What the fake auth slice answers: a team row, the key's own access check and the denial text
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct AccessScript {
    pub(super) database: bool,
    pub(super) team: Lookup,
    pub(super) team_models: Vec<String>,
    pub(super) key: Outcome,
    pub(super) message: String,
}

// Which managed-files dependency the fake proxy module withholds
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ManagedScript {
    pub(super) logging: Option<bool>,
    pub(super) hook: Option<Hook>,
    pub(super) router: Option<bool>,
}

// The content a provider or managed read hands back
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ContentScript {
    pub(super) contents: Option<String>,
    pub(super) not_bytes: Option<String>,
    pub(super) http: Option<HttpScript>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HttpScript {
    pub(super) status: u16,
    #[serde(default)]
    pub(super) detail: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) file_id: String,
    #[serde(default)]
    pub(super) custom_llm_provider: BatchInputProvider,
    #[serde(default)]
    pub(super) caller: Option<BatchLimitPrincipal>,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub(super) data: RequestData,
    #[serde(default)]
    pub(super) descriptors: Vec<BatchLimitDescriptor>,
    #[serde(default)]
    pub(super) content: ContentScript,
    #[serde(default)]
    pub(super) managed: ManagedScript,
    #[serde(default)]
    pub(super) access: AccessScript,
    #[serde(default)]
    pub(super) tokens: Vec<Option<i64>>,
}
