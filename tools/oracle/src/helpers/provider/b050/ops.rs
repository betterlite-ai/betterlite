use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::agent_platform_dispatch::context::{DispatchContext, DispatchGlobals};
use betterlite_provider::agent_platform_dispatch::error::DispatchError;
use betterlite_provider::agent_platform_dispatch::table::dispatch;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::fixture::{Handler, NoEntra, Registry, secret_lookup};
use super::script::{DispatchCase, Platform};

pub(super) fn dispatch_case(case: &DispatchCase) -> HelperResult {
    let secrets = secret_lookup();
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let global_key = case.globals.api_key.as_deref().map(SecretString::from);
    let litellm_params = Object::new();
    let registry = Registry::new(&case.registry);
    let handler = Handler::new(case.platform, &case.upstream)?;
    let context = DispatchContext {
        acompletion: false,
        api_base: case.api_base.as_deref(),
        api_key: api_key.as_ref(),
        custom_llm_provider: slug(case.platform),
        headers: case.headers.as_ref(),
        litellm_params: &litellm_params,
        messages: &case.messages,
        model: &case.model,
        optional_params: &case.optionals,
        provider_config: Some(tag(case.platform)),
        stream: case.stream,
        timeout: Some(Duration::from_secs(600)),
        globals: DispatchGlobals {
            api_base: case.globals.api_base.as_deref(),
            api_key: global_key.as_ref(),
            headers: case.globals.headers.as_ref(),
        },
        secrets: &secrets,
        environment: &case.environment,
        registry: Some(&registry),
        entra: &NoEntra,
    };
    let Some(outcome) = dispatch(tag(case.platform), &context, &handler) else {
        return missing_arm();
    };
    Ok(outcome_value(handler.requests(), outcome))
}

// the dispatch table always has an arm for the three tags this helper hands it
fn missing_arm() -> HelperResult {
    Err(Box::new(HelperError {
        class: String::from("NoDispatchArm"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: String::from("the dispatch table has no arm for this platform"),
    }))
}

fn outcome_value(requests: Vec<Value>, outcome: Result<Value, DispatchError>) -> Value {
    let request = requests.into_iter().next();
    match outcome {
        Ok(reply) => json!({"request": request, "reply": reply, "error": Value::Null}),
        Err(failure) => json!({"request": request, "reply": null, "error": error_value(&failure)}),
    }
}

fn error_value(failure: &DispatchError) -> Value {
    match failure {
        DispatchError::Handler(handler) => {
            json!({"status": handler.status_code, "message": handler.message})
        }
        DispatchError::A2a(_)
        | DispatchError::LangGraph(_)
        | DispatchError::LangFlow(_)
        | DispatchError::Environment { .. }
        | DispatchError::MissingA2aApiBase => {
            json!({"status": null, "message": failure.to_string()})
        }
    }
}

const fn slug(platform: Platform) -> &'static str {
    match platform {
        Platform::A2a => "a2a",
        Platform::Langgraph => "langgraph",
        Platform::Langflow => "langflow",
    }
}

const fn tag(platform: Platform) -> ChatConfigTag {
    match platform {
        Platform::A2a => ChatConfigTag::A2a,
        Platform::Langgraph => ChatConfigTag::LangGraph,
        Platform::Langflow => ChatConfigTag::LangFlow,
    }
}
