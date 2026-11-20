use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Payload {
        kwargs: JsonObject,
        #[serde(default)]
        response: ResponseInput,
        start_time: String,
        end_time: String,
        #[serde(default)]
        settings: SettingsInput,
        #[serde(default)]
        router: Option<RouterInput>,
        #[serde(default)]
        clock: f64,
        session_uuid: String,
    },
    EndUser {
        litellm_params: JsonObject,
        service: String,
        #[serde(default)]
        settings: SettingsInput,
    },
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseInput {
    #[default]
    Absent,
    Other,
    UnknownModelError,
    Mapping(JsonObject),
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct RouterInput {
    pub(crate) model_list: Vec<RouterDeployment>,
    // Router.model_names, which is_recognized_model reads
    pub(crate) recognized: Vec<String>,
    // Router.team_public_model_names
    pub(crate) team_public: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct RouterDeployment {
    pub(crate) model_name: String,
    #[serde(default)]
    pub(crate) litellm_params: JsonObject,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsInput {
    pub(crate) disable_end_user_cost_tracking: Option<bool>,
    #[serde(rename = "enable_end_user_cost_tracking_prometheus_only")]
    pub(crate) enable_prometheus_only: Option<bool>,
    pub(crate) store_bodies: bool,
    pub(crate) max_string_length: Option<i64>,
    pub(crate) turn_off_message_logging: bool,
    pub(crate) missing_session_id: Option<String>,
}
