use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::prompt_dispatch::identity::PromptIdentity;
use crate::prompt_dispatch::ports::CompiledPrompt;

// The keys LiteLLM strips from the deployment params before it merges, because it stamps them again after.
const PROMPT_MANAGEMENT_PARAMS: [&str; 6] = [
    "bitbucket_config",
    "dotprompt_config",
    "prompt_id",
    "prompt_variables",
    "prompt_label",
    "prompt_version",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptCall {
    Direct,
    WithFallbacks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalPromptRequest {
    pub fields: JsonObject,
    pub call: PromptCall,
}

// The request Router._prompt_management_factory hands to the next call: deployment params, then the request, then the compiled optional params.
#[must_use]
pub fn merge_prompt_request(
    data: &JsonObject,
    request: &JsonObject,
    compiled: &CompiledPrompt,
    identity: &PromptIdentity,
    new_model_has_deployments: bool,
) -> FinalPromptRequest {
    let mut merged = prompt_free_data(data);
    for source in [request, &compiled.optional_params] {
        merged.extend(
            source
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    merged.insert("model".to_owned(), Json::Text(compiled.model.clone()));
    merged.insert("messages".to_owned(), Json::List(compiled.messages.clone()));
    if let Some(logging) = request
        .get("litellm_logging_obj")
        .filter(|value| !matches!(value, Json::Null))
    {
        merged.insert("litellm_logging_obj".to_owned(), logging.clone());
    }
    merged.insert(
        "prompt_id".to_owned(),
        identity.prompt_id.clone().unwrap_or(Json::Null),
    );
    merged.insert(
        "prompt_variables".to_owned(),
        identity.prompt_variables.clone().unwrap_or(Json::Null),
    );
    merged.insert(
        "prompt_label".to_owned(),
        identity.prompt_label.clone().unwrap_or(Json::Null),
    );
    let call = if new_model_has_deployments {
        PromptCall::WithFallbacks
    } else {
        merged.shift_remove("original_function");
        PromptCall::Direct
    };
    FinalPromptRequest {
        fields: merged,
        call,
    }
}

fn prompt_free_data(data: &JsonObject) -> JsonObject {
    let mut kept = data.clone();
    kept.retain(|key, _| !PROMPT_MANAGEMENT_PARAMS.contains(&key.as_str()));
    kept
}
