use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) catalog: Vec<CatalogModel>,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogModel {
    pub(crate) key: String,
    pub(crate) model_info: Box<RawValue>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    SupportedParams {
        model: String,
    },
    MapParams {
        model: String,
        #[serde(default)]
        non_default_params: Object,
        #[serde(default)]
        ordered_params: Vec<(String, Value)>,
        #[serde(default)]
        optional_params: Object,
    },
    MapExtraBody {
        model: String,
        #[serde(default)]
        optional_params: Object,
    },
    Config {
        #[serde(default)]
        settings: Object,
    },
}
