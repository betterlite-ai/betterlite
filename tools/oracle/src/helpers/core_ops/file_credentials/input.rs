use betterlite_core::api::file_credentials::hints::ModelHints;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    Extract {
        file_id: String,
        hints: ModelHints,
    },
    Routing {
        file_id: String,
        hints: ModelHints,
        check_file_id_encoding: bool,
        #[serde(default)]
        router: Option<RouterInput>,
    },
    TeamCredentials {
        provider: String,
        key: KeyInput,
        #[serde(default)]
        router: Option<RouterInput>,
    },
    ApplyTeamCredentials {
        provider: String,
        key: KeyInput,
        data: JsonObject,
        #[serde(default)]
        router: Option<RouterInput>,
    },
    InternalCredentials {
        model_id: Option<String>,
        data: JsonObject,
        router: RouterInput,
    },
    DeploymentInfo {
        model_id: String,
        data: JsonObject,
        #[serde(default)]
        router: Option<RouterInput>,
    },
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyInput {
    pub(crate) models: Vec<String>,
    pub(crate) team_models: Vec<String>,
    pub(crate) team_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowInput {
    pub(crate) model_name: String,
    pub(crate) model_info: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouterInput {
    pub(crate) model_names: Vec<String>,
    pub(crate) access_groups: IndexMap<String, Vec<String>>,
    pub(crate) deployments: Vec<RowInput>,
    pub(crate) credentials: IndexMap<String, JsonObject>,
    pub(crate) credential_deployments: IndexMap<String, JsonObject>,
}

#[derive(Clone, Serialize)]
pub(crate) struct Lookup {
    pub(crate) model_id: String,
    pub(crate) team_id: Option<String>,
}
