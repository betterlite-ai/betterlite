use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) triggers: Option<Vec<TriggerCall>>,
    pub(crate) repairs: Option<Vec<RepairCall>>,
    #[serde(default)]
    pub(crate) bounds: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TriggerCall {
    pub(crate) error: String,
    pub(crate) request_drop_params: bool,
    pub(crate) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepairCall {
    pub(crate) error: String,
    pub(crate) body: Object,
}
