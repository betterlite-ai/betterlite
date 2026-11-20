use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::hosted_platform_dispatch::call::EmbeddingCall;
use betterlite_provider::hosted_platform_dispatch::context::DispatchSettings;
use betterlite_provider::hosted_platform_dispatch::embedding::dispatch;
use secrecy::SecretString;
use serde_json::json;

use super::fixtures::{environment, section_settings};
use super::input::{Case, as_object};
use super::input_failure;
use super::wire::request_view;

const EMBEDDING_ENDPOINT: &str = "/embeddings";
const FIREWORKS_AI_API_KEY: &str = "FIREWORKS_AI_API_KEY";

pub(super) fn run(case: &Case) -> HelperResult {
    let env = environment(case);
    let settings = section_settings(case, &env);
    let call = embedding_call(case, &env)?;
    let resolved = dispatch(
        case.provider.provider_id(),
        &call,
        DispatchSettings::from(&settings),
    )
    .ok_or_else(|| {
        input_failure(format!(
            "{} has no embedding arm of its own",
            case.provider.slug()
        ))
    })?
    .map_err(|error| input_failure(error.to_string()))?;
    let base = resolved
        .api_base
        .as_deref()
        .ok_or_else(|| input_failure(String::from("the embedding arm resolved no api_base")))?;
    let url = format!("{base}{EMBEDDING_ENDPOINT}");
    Ok(json!({
        "request": request_view(&url, resolved.api_key.as_ref(), resolved.headers.as_ref()),
    }))
}

fn embedding_call(
    case: &Case,
    env: &BTreeMap<String, String>,
) -> Result<EmbeddingCall, Box<HelperError>> {
    let input = case
        .input
        .clone()
        .ok_or_else(|| input_failure(String::from("the embedding case names no input")))?;
    Ok(EmbeddingCall {
        model: case.model.clone(),
        input: EmbeddingInput::Text(input),
        optional_params: as_object(case.optional_params.as_ref())?,
        litellm_params: Object::new(),
        // get_llm_provider fills the key from the environment before main.py's arm runs
        api_key: case
            .api_key
            .as_deref()
            .filter(|key| !key.is_empty())
            .or_else(|| env.get(FIREWORKS_AI_API_KEY).map(String::as_str))
            .map(SecretString::from),
        api_base: case.api_base.clone(),
        headers: case.headers.clone(),
        // litellm.embedding()'s default request timeout
        timeout: Duration::from_secs(600),
        aembedding: false,
    })
}
