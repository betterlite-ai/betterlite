use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) guardrail: GuardrailScript,
    pub(super) translation: TranslationScript,
    pub(super) chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GuardrailScript {
    pub(super) kind: Kind,
    pub(super) name: String,
    pub(super) texts_out: Option<Vec<String>>,
    pub(super) tool_calls_out: Option<Vec<GuardrailValue>>,
    pub(super) hook: HookScript,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Unified,
    Legacy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HookScript {
    pub(super) mode: HookMode,
    pub(super) response: Option<Assembled>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum HookMode {
    Nothing,
    Replace,
    Mutate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Assembled {
    pub(super) texts: Vec<String>,
    pub(super) tool_calls: Vec<GuardrailValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TranslationScript {
    pub(super) delivers: bool,
    pub(super) tool_calls: Option<Vec<GuardrailValue>>,
    pub(super) rescans: bool,
}
