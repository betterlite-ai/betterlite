use betterlite_openai::openai_base_config::{Headers, Object};
use serde::Deserialize;
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Environment {
    pub(super) region_name: Option<String>,
    pub(super) runtime_endpoint: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub(crate) enum Input {
    Url {
        #[serde(default)]
        environment: Environment,
        runs: Vec<UrlRun>,
    },
    Sign {
        bearer: Option<String>,
        runs: Vec<SignRun>,
    },
    Reply {
        runs: Vec<ReplyRun>,
    },
    Collect {
        runs: Vec<CollectRun>,
        stream: String,
    },
}
#[derive(Debug, Deserialize)]
pub(crate) struct UrlRun {
    pub(super) model: String,
    pub(super) endpoint: String,
    pub(super) region: Option<String>,
    pub(super) model_id: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) runtime_endpoint: Option<String>,
    pub(super) query: Option<Object>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct SignRun {
    #[serde(default)]
    pub(super) headers: Headers,
    pub(super) data: Option<Object>,
}
#[derive(Debug, Deserialize)]
pub(crate) struct ReplyRun {
    pub(super) provider: String,
    pub(super) endpoint: String,
    pub(super) body: String,
}
#[derive(Debug, Deserialize)]
pub(crate) struct CollectRun {
    pub(super) model: String,
    pub(super) endpoint: String,
}
