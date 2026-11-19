mod handlers;
mod scripted;

use std::collections::BTreeMap;

use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::responses::ResponsesInput;
use betterlite_prompts::file_ids::tools::ResponsesTool;
use serde::Deserialize;

use scripted::Hook;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Call {
    Selection(Selection),
    Prefixes(Prefixes),
    Compatible(Compatible),
    CredentialBoundary(CredentialBoundary),
    PromptCall(PromptCall),
    FileMapping(FileMapping),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    provider: String,
    model: String,
    use_chat_completions_api: bool,
    model_info: JsonValue,
    kwargs: ParameterMap,
    allowed_openai_params: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prefixes {
    model: String,
    flag_present: bool,
    flag: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compatible {
    environment: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialBoundary {
    original_model: String,
    swapped_model: String,
    provider: Option<String>,
    prompt_id: Option<String>,
    kwargs: ParameterMap,
    model_info: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptCall {
    input: JsonValue,
    model: String,
    provider: Option<String>,
    use_chat_completions_api: bool,
    logging: bool,
    hook: Option<Hook>,
    async_outcome: Option<JsonValue>,
    kwargs: ParameterMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMapping {
    input: ResponsesInput,
    tools: Option<Vec<ResponsesTool>>,
    model_info: Option<JsonValue>,
    mapping: Option<FileIdMapping>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Call::Selection(case) => handlers::selection(case),
            Call::Prefixes(case) => handlers::prefixes(case),
            Call::Compatible(case) => handlers::compatible(case),
            Call::CredentialBoundary(case) => handlers::credential_boundary(case),
            Call::PromptCall(case) => handlers::prompt_call(case),
            Call::FileMapping(case) => handlers::file_mapping(case),
        }
    }
}
