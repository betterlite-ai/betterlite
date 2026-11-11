use std::collections::BTreeSet;

use betterlite_models::params::CallType;
use betterlite_models::params::capabilities::{OpenAiCapability, OpenAiConfigTag};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

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
    #[serde(default)]
    pub(crate) fallback_generalizations: Option<Vec<Value>>,
    #[serde(default)]
    pub(crate) cache_maxsize: Option<i64>,
    #[serde(default)]
    pub(crate) dynamic: Option<DynamicScript>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DynamicScript {
    Value { body: Value },
    Error,
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
        parameters: Option<ParameterScript>,
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
    Clear,
    Replace {
        key: String,
        entry: Value,
    },
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ParameterScript {
    Value {
        #[serde(default)]
        body: Option<Vec<String>>,
    },
    Absent,
    Error,
    B175 {
        call_type: CallType,
        config: OpenAiConfigTag,
        #[serde(default)]
        capabilities: BTreeSet<OpenAiCapability>,
        #[serde(default)]
        base_model: Option<String>,
        #[serde(default)]
        base_capabilities: Option<BTreeSet<OpenAiCapability>>,
    },
}
