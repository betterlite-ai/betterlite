use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

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

// How the call relates to a router: absent, built from the snapshot, or built over a broken pattern table.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RouterCall {
    Missing,
    Ready,
    BrokenPattern,
}

// The call's own arguments, the router flags resolved to the value the source's signature gives an absent one.
#[derive(Debug, Deserialize)]
#[serde(from = "RawArguments")]
pub(crate) struct Arguments {
    pub(crate) model_names: Vec<String>,
    pub(crate) user_role: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) healthy_only: Option<bool>,
    pub(crate) model_list_healthy_only: bool,
    pub(crate) router: RouterCall,
    pub(crate) health: Health,
}

// The arguments as a case writes them, where absent and null both mean the source's default.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawArguments {
    model_names: Vec<String>,
    user_role: Option<String>,
    team_id: Option<String>,
    healthy_only: Option<bool>,
    model_list_healthy_only: Option<bool>,
    with_router: Option<bool>,
    health: Health,
    broken_pattern: bool,
}

impl Default for Arguments {
    fn default() -> Self {
        Self::from(RawArguments::default())
    }
}

impl From<RawArguments> for Arguments {
    fn from(raw: RawArguments) -> Self {
        let router = if matches!(raw.with_router, Some(false)) {
            RouterCall::Missing
        } else if raw.broken_pattern {
            RouterCall::BrokenPattern
        } else {
            RouterCall::Ready
        };
        Self {
            model_names: raw.model_names,
            user_role: raw.user_role,
            team_id: raw.team_id,
            healthy_only: raw.healthy_only,
            model_list_healthy_only: raw.model_list_healthy_only.unwrap_or(false),
            router,
            health: raw.health,
        }
    }
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
