use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;
use serde::Deserializer;

// One helper_case call: the router snapshot, the caller and the function the case reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    #[serde(default)]
    pub(crate) model_list: Vec<Row>,
    #[serde(default)]
    pub(crate) model_group_alias: JsonObject,
    #[serde(default)]
    pub(crate) arguments: Arguments,
}

// A row without model_info declares none, which no key of the mapping can match.
const fn default_model_info() -> Json {
    Json::Null
}

// Router(model_list=...): one deployment as the caller declares it, its model_info left as raw JSON for the non-mapping case.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) litellm_params: JsonObject,
    #[serde(default = "default_model_info")]
    pub(crate) model_info: Json,
}

// The call's own arguments; every one of them takes the value the source's signature gives an absent one.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Arguments {
    pub(crate) model_names: Vec<String>,
    pub(crate) user_role: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) healthy_only: Option<bool>,
    #[serde(deserialize_with = "null_as_false")]
    pub(crate) model_list_healthy_only: bool,
    #[serde(default = "with_router_default", deserialize_with = "null_as_true")]
    pub(crate) with_router: bool,
    pub(crate) health: Health,
    pub(crate) broken_pattern: bool,
}

// A null bool takes the value the source's signature gives an absent one.
fn null_as_false<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or_default())
}

fn null_as_true<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(true))
}

const fn with_router_default() -> bool {
    true
}

// B307's answer, injected so a case can make the cache refuse the read.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Health {
    pub(crate) unhealthy_names: Vec<String>,
    pub(crate) fails: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    UndiscoverableModelNames,
    DiscoverableRows,
    HiddenUnhealthyModelNames,
}
