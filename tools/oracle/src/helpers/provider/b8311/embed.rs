use std::time::Duration;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::gateway_provider_dispatch::call::EmbeddingCall;
use betterlite_provider::gateway_provider_dispatch::context::DispatchSettings;
use betterlite_provider::gateway_provider_dispatch::embedding::dispatch;
use serde_json::json;

use super::fixtures::{environment, section_settings};
use super::input::{Case, PreResolved, as_object, pre_resolved};
use super::input_failure;
use super::wire::request_view;

const EMBEDDING_ENDPOINT: &str = "/embeddings";

// the request the arm reads: the case's fields plus litellm.embedding()'s default timeout
fn request(case: &Case, pre: PreResolved) -> Result<EmbeddingCall, Box<HelperError>> {
    let input = case
        .input
        .clone()
        .ok_or_else(|| input_failure(String::from("the embedding case names no input")))?;
    Ok(EmbeddingCall {
        model: case.model.clone(),
        input: EmbeddingInput::Text(input),
        optional_params: as_object(case.optional_params.as_ref())?,
        litellm_params: Object::new(),
        api_key: pre.api_key,
        api_base: pre.api_base,
        headers: case.headers.clone(),
        timeout: Duration::from_secs(600),
        aembedding: false,
    })
}

pub(super) fn run(case: &Case) -> HelperResult {
    let env = environment(case);
    let settings = section_settings(case, &env);
    let call = request(
        case,
        pre_resolved(
            case.provider,
            case.api_base.as_ref(),
            case.api_key.as_ref(),
            &env,
        ),
    )?;
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
    })?;
    let base = resolved
        .api_base
        .as_deref()
        .ok_or_else(|| input_failure(String::from("the embedding arm resolved no api_base")))?;
    let url = format!("{base}{EMBEDDING_ENDPOINT}");
    Ok(json!({
        "request": request_view(&url, resolved.api_key.as_ref(), resolved.headers.as_ref()),
    }))
}
