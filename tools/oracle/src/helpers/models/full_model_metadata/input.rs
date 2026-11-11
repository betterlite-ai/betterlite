use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use betterlite_config::sections::full_model_metadata::DEFAULT_MAX_LRU_CACHE_SIZE;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) fixture: FixtureInput,
    pub(crate) steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureInput {
    #[serde(default)]
    pub(crate) model_cost: IndexMap<String, Value>,
    #[serde(default = "default_cache_maxsize")]
    pub(crate) cache_maxsize: i64,
    #[serde(default)]
    pub(crate) dynamic: Option<Value>,
}

const fn default_cache_maxsize() -> i64 {
    DEFAULT_MAX_LRU_CACHE_SIZE
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Call {
        model: String,
        #[serde(default)]
        custom_llm_provider: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        parameters: ParameterScript,
    },
    HelperCall {
        model: String,
        #[serde(default)]
        custom_llm_provider: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
    },
    Mutate {
        record: usize,
        field: String,
        value: Value,
    },
    Invalidate,
    Replace {
        key: String,
        entry: Value,
    },
}

#[derive(Clone, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ParameterScript {
    Value {
        #[serde(default)]
        body: Option<Vec<String>>,
    },
    #[default]
    Absent,
    Error,
}
