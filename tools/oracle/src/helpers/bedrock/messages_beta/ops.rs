use std::collections::BTreeSet;
use std::fmt::Display;

use betterlite_bedrock::bedrock_messages_beta::beta::BetaHeadersRequest;
use betterlite_bedrock::bedrock_messages_beta::context_management::filter_context_management;
use betterlite_bedrock::bedrock_messages_beta::fields::strip_unsupported_fields;
use betterlite_bedrock::bedrock_messages_beta::provider::{
    CUSTOM_LLM_PROVIDER, beta_headers_provider,
};
use betterlite_bedrock::bedrock_messages_beta::web_search::reject_web_search_tools;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_prompts::arguments::json::Object;
use serde_json::json;
use tracing::Level;

use super::fixture::{BUNDLED_BETA_CONFIG, Fixture, FixtureError, Session};
use super::input::{BetaHeadersCall, ContextCase, FieldsCase, IdentityCase, Input};
use super::logs::capture_logs;
use crate::helpers::{HelperError, HelperResult};

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockMessagesBeta".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn beta_headers(model_cost: &ParamMap, calls: &[BetaHeadersCall]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let matrix: BetaMatrix = serde_json::from_str(BUNDLED_BETA_CONFIG)
        .map_err(FixtureError::Json)
        .map_err(failure)?;
    let mut session = Session::new(&fixture).map_err(failure)?;
    let (_guard, logs) = capture_logs();
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let mut outgoing = call.outgoing.clone();
        let request = BetaHeadersRequest {
            model: &call.model,
            messages: &call.messages,
            tools: &call.tools,
            mcp_servers: &call.mcp_servers,
            safeguards: call.safeguards.as_ref(),
            params: &call.params,
            headers: &call.headers,
            outgoing: &mut outgoing,
            clear_thinking_injected: call.clear_thinking_injected,
            subclass_provider: call.subclass_provider.as_deref(),
        };
        let betas = session.betas(request, &matrix).map_err(failure)?;
        answers.push(json!({
            "betas": betas,
            "warnings": logs.take(Level::WARN),
        }));
    }
    Ok(json!({ "calls": answers }))
}

fn context_filter(cases: &[ContextCase]) -> HelperResult {
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        let mut request = case.request.clone();
        let mut beta_set: BTreeSet<String> = case.betas.iter().cloned().collect();
        filter_context_management(&mut request, &mut beta_set);
        answers.push(json!({
            "request": request,
            "betas": beta_set.into_iter().collect::<Vec<_>>(),
        }));
    }
    Ok(json!({ "cases": answers }))
}

fn web_search(request: &Object, model: &str) -> HelperResult {
    match reject_web_search_tools(request, model) {
        Ok(()) => Ok(json!({ "accepted": true })),
        Err(error) => Err(Box::new(HelperError {
            class: "BadRequestError".to_owned(),
            status: Some(error.status_code()),
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

fn unsupported_fields(cases: &[FieldsCase]) -> HelperResult {
    let (_guard, logs) = capture_logs();
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        let kept = strip_unsupported_fields(&case.request).map_err(failure)?;
        answers.push(json!({
            "request": kept,
            "logs": logs.take(Level::DEBUG),
        }));
    }
    Ok(json!({ "cases": answers }))
}

fn provider_identity(model_cost: &ParamMap, cases: &[IdentityCase]) -> HelperResult {
    let fixture = Fixture::load(model_cost).map_err(failure)?;
    let mut session = Session::new(&fixture).map_err(failure)?;
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        answers.push(json!({
            "custom_llm_provider": CUSTOM_LLM_PROVIDER,
            "beta_headers_provider": beta_headers_provider(case.subclass_provider.as_deref()),
            "supports_tool_search": session.supports_tool_search(&case.model),
        }));
    }
    Ok(json!({ "cases": answers }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::BetaHeaders { model_cost, calls } => beta_headers(model_cost, calls),
        Input::ContextFilter { cases } => context_filter(cases),
        Input::WebSearch { request, model } => web_search(request, model),
        Input::UnsupportedFields { cases } => unsupported_fields(cases),
        Input::ProviderIdentity { model_cost, cases } => provider_identity(model_cost, cases),
    }
}
