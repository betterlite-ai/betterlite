use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

// One helper_case call: the Router the case builds, its credential_list and the method it reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) credentials: Vec<CredentialRow>,
    // pattern deployments a case adds after the Router is built, which its config path would have validated
    #[serde(default)]
    pub(crate) pattern_deployments: Vec<PatternRow>,
    #[serde(default)]
    pub(crate) arguments: Arguments,
}

// Router.pattern_router.add_pattern(pattern, deployment): a raw pattern deployment the case injects.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PatternRow {
    pub(crate) pattern: String,
    pub(crate) deployment: Row,
}

// Router(model_list=...): one deployment as the caller declares it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    pub(crate) litellm_params: JsonObject,
    #[serde(default)]
    pub(crate) model_info: JsonObject,
}

// A litellm.credential_list entry: LiteLLM's CredentialItem, minus the credential_info this task never reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRow {
    pub(crate) credential_name: String,
    pub(crate) credential_values: JsonObject,
}

// The method's own arguments.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Arguments {
    pub(crate) model_id: Option<String>,
    pub(crate) team_id: Option<String>,
}

// The Router methods this operation reads, by the name the case calls.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    GetDeploymentCredentials,
    #[serde(rename = "get_deployment_credentials_with_provider")]
    CredentialsWithProvider,
}
