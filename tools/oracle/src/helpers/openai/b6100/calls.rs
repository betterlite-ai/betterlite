use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use betterlite_core::edenai_messages::{
    EnvironmentInput, descriptor, validate_environment as messages_validate_environment,
};
use betterlite_core::json_messages_policy::{JsonMessagesPolicy, MessagesCredentials};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::edenai_chat::{
    CatalogReply, ReasoningLookup, TransformInput, get_models, models_url, supported_params,
    transform_request,
};
use betterlite_openai::edenai_common::{require_api_key, resolve_api_base, resolve_api_key};
use betterlite_openai::edenai_embedding::{
    TransformInput as EmbeddingInput, map_openai_params, plan_embedding,
};
use betterlite_openai::edenai_responses::{
    PrepareInput, SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET, prepare,
};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::input::{Call, CatalogCase, ChatParams, ChatRequest, EmbeddingPlan};
use super::ops::{MODEL, env_with};

struct FixedReasoning(bool);

impl ReasoningLookup for FixedReasoning {
    type Error = Infallible;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Self::Error> {
        Ok(self.0)
    }
}

pub(super) fn credentials_value(case: &Call) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.clone().map(SecretString::from);
    let required = match require_api_key(explicit.as_ref(), MODEL, &env) {
        Ok(found) => json!({"key": found.expose_secret(), "status": null}),
        Err(error) => json!({"key": null, "status": error.status_code}),
    };
    Ok(json!({
        "api_base": resolve_api_base(case.api_base.as_deref(), &env),
        "api_key": resolve_api_key(explicit.as_ref(), &env)
            .map(|found| found.expose_secret().to_owned()),
        "required": required,
    }))
}

pub(super) fn chat_params_value(case: &ChatParams) -> Result<Value, Box<HelperError>> {
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let lookup = FixedReasoning(case.reasoning);
    Ok(json!(supported_params(&lookup, &case.model, &catalog)))
}

pub(super) fn chat_request_value(case: &ChatRequest) -> Result<Value, Box<HelperError>> {
    let messages: [ChatMessage; 0] = [];
    let environment = BTreeMap::new();
    let downloads = RefuseDownloads;
    let body = transform_request(
        &TransformInput {
            model: &case.model,
            messages: &messages,
            optional_params: &case.optional_params,
            environment: &environment,
            downloads: &downloads,
        },
        Mode::Sync,
    )
    .or_value_error()?;
    serde_json::to_value(body).or_value_error()
}

pub(super) fn catalog_value(case: &CatalogCase) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, None);
    let url = models_url(case.api_base.as_deref(), &env);
    let reply = CatalogReply {
        status: case.status,
        headers: &case.headers,
        text: &case.text,
    };
    match get_models(&reply) {
        Ok(models) => Ok(json!({"url": url, "models": models})),
        Err(error) => Ok(json!({"url": url, "status": error.status_code})),
    }
}

pub(super) fn embedding_plan_value(case: &EmbeddingPlan) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.clone().map(SecretString::from);
    let plan = plan_embedding(&EmbeddingInput {
        model: &case.model,
        input: &case.input,
        optional_params: &case.optional_params,
        headers: &case.headers,
        api_key: explicit.as_ref(),
        api_base: case.api_base.as_deref(),
        env: &env,
    })
    .or_value_error()?;
    let params = map_openai_params(&case.non_default_params, &case.optional_params);
    Ok(json!({
        "params": params,
        "url": plan.url,
        "headers": plan.headers,
        "body": plan.body,
    }))
}

pub(super) fn prepare_value(case: &Call) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.clone().map(SecretString::from);
    let prepared = prepare(PrepareInput {
        headers: case.headers.clone(),
        api_key: explicit.as_ref(),
        model: MODEL,
        api_base: case.api_base.as_deref(),
        env: &env,
    });
    match prepared {
        Ok(found) => Ok(json!({
            "url": found.url,
            "headers": found.headers,
            "should_fake_stream": SHOULD_FAKE_STREAM,
            "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        })),
        Err(error) => Ok(json!({"status": error.status_code})),
    }
}

pub(super) fn messages_environment_value(case: &Call) -> Result<Value, Box<HelperError>> {
    let env = env_with(&case.environment, case.global_api_key.as_deref());
    let explicit = case.api_key.clone().map(SecretString::from);
    let lookup = case.environment.clone();
    let optional = Object::new();
    let policy = JsonMessagesPolicy {
        descriptor: descriptor(),
    };
    let validated = messages_validate_environment(
        &policy,
        EnvironmentInput {
            headers: case.headers.clone(),
            api_key: explicit.as_ref(),
            model: MODEL,
            env: &env,
            lookup: &lookup,
            optional_params: &optional,
            messages: &[],
        },
    );
    let headers = match validated {
        Ok(found) => found,
        Err(error) => return Ok(json!({"status": error.status_code})),
    };
    let url = policy
        .complete_url(&MessagesCredentials {
            api_base: case.api_base.as_deref(),
            api_key: None,
            global_api_key: None,
            lookup: &lookup,
        })
        .or_value_error()?;
    Ok(json!({"headers": headers, "url": url}))
}
