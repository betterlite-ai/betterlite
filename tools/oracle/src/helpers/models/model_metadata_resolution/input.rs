use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scenario {
    PotentialNames,
    ProviderAliases,
    DynamicProviderMetadata,
    OrderedExactLookup,
    FillMissingExactEntry,
    GeneralizationOnlyUnknown,
    UnmappedErrorBoundary,
    HuggingfaceConfigMetadata,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fixture {
    #[serde(default)]
    pub(crate) model_cost: IndexMap<String, Box<RawValue>>,
    #[serde(default)]
    pub(crate) fallback_generalizations: Option<Vec<Box<RawValue>>>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) known_model_names: Vec<String>,
    #[serde(default)]
    pub(crate) custom_providers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Query {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) dynamic: Option<DynamicScript>,
    #[serde(default)]
    pub(crate) hf: Option<HfScript>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DynamicKind {
    Value,
    None,
    Error,
}

// A plain struct, because a RawValue field inside an internally tagged enum cannot deserialize.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DynamicScript {
    pub(crate) kind: DynamicKind,
    #[serde(default)]
    pub(crate) body: Option<Box<RawValue>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HfScript {
    Status { status: u16, body: String },
    TransportError,
    Timeout,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenario: Scenario,
    #[serde(default)]
    pub(crate) fixture: Fixture,
    pub(crate) queries: Vec<Query>,
}
