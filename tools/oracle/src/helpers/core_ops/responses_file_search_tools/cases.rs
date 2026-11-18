use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResult;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolSchemaCase {
    pub(crate) vector_store_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolReplacementCase {
    #[serde(default)]
    pub(crate) tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultRenderingCase {
    pub(crate) results: Vec<VectorStoreSearchResult>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputItemsCase {
    pub(crate) call_id: String,
    pub(crate) queries: Vec<String>,
    #[serde(default)]
    pub(crate) include_search_results: bool,
    #[serde(default)]
    pub(crate) results: Option<Vec<VectorStoreSearchResult>>,
    pub(crate) text: String,
}

// the original response as LiteLLM's model holds it, with _hidden_params as a private attribute
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginalView {
    pub(crate) id: String,
    pub(crate) created_at: i64,
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) usage: Option<ResponseApiUsage>,
    #[serde(default, rename = "_hidden_params")]
    pub(crate) hidden_params: Option<HiddenParams>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseSynthesisCase {
    pub(crate) call_id: String,
    pub(crate) queries: Vec<String>,
    #[serde(default)]
    pub(crate) include_search_results: bool,
    pub(crate) results: Vec<VectorStoreSearchResult>,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) original: Option<OriginalView>,
    #[serde(default, rename = "_hidden_params")]
    pub(crate) first_hidden: Option<HiddenParams>,
    pub(crate) uuid: String,
    pub(crate) now: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArgumentPreparationCase {
    pub(crate) prepares: Vec<Fields>,
    pub(crate) fallback_call_id: String,
    pub(crate) tool_calls: Vec<Fields>,
    pub(crate) query_args: Vec<Fields>,
    pub(crate) query_inputs: Vec<Json>,
}

// boxed so every variant carries a pointer, well within the 3x variant_size_differences enforces
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ToolSchema(Box<ToolSchemaCase>),
    ToolReplacement(Box<ToolReplacementCase>),
    ResultRendering(Box<ResultRenderingCase>),
    OutputItems(Box<OutputItemsCase>),
    ResponseSynthesis(Box<ResponseSynthesisCase>),
    ArgumentPreparation(Box<ArgumentPreparationCase>),
}
