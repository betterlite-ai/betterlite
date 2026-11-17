use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ToolChoice(ToolChoiceInput),
    Detector(DetectorInput),
    Separate(SeparateInput),
    Transform(TransformInput),
    JsonTool(JsonToolInput),
    ApplyTools(ApplyToolsInput),
    Translate(TranslateInput),
    Additional(AdditionalInput),
    OutputConfig(OutputConfigInput),
    ToolSpec(ToolSpecInput),
    MediaTypes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolChoiceInput {
    pub(crate) model: String,
    pub(crate) model_cost: Json,
    pub(crate) tool_choice: Json,
    pub(crate) global_drop: bool,
    pub(crate) call_drop: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetectorInput {
    pub(crate) tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeparateInput {
    pub(crate) tools: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformInput {
    pub(crate) tools: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JsonToolInput {
    pub(crate) json_schema: Option<Json>,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyToolsInput {
    pub(crate) tools: Vec<Json>,
    pub(crate) model: String,
    pub(crate) stream: Json,
    pub(crate) optional_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TranslateInput {
    pub(crate) model: String,
    pub(crate) model_cost: Json,
    pub(crate) value: Json,
    pub(crate) stream: Json,
    pub(crate) is_thinking_enabled: bool,
    pub(crate) optional_params: Params,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdditionalInput {
    pub(crate) schema: Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputConfigInput {
    pub(crate) json_schema: Option<Json>,
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolSpecInput {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Json,
    pub(crate) strict: Option<bool>,
    pub(crate) supports_strict_tools: bool,
}

pub(crate) type Params = IndexMap<String, Json>;
