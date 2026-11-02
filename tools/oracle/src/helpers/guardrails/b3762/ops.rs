use betterlite_guardrails::cisco_configuration::records::Direction;
use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_conversation::request::request_messages;
use betterlite_guardrails::cisco_inspection::chat::{ChatInspectionArgs, inspect_chat};
use betterlite_guardrails::cisco_inspection::hooks::{
    HookArgs, PostCallArgs, async_moderation_hook, async_post_call_success_hook,
    async_pre_call_hook,
};
use betterlite_guardrails::cisco_inspection::mcp::{inspect_mcp_request, inspect_mcp_response};
use betterlite_guardrails::cisco_inspection::transport::{build_headers, post_inspection};
use betterlite_guardrails::cisco_verdict_enforcement::finalize::ResponseSurface;
use serde_json::{Value, json};

use super::support::{
    applied_guardrails, configure, fail, guardrail, json_to_value, outcome_tag, prepare,
    recorded_call,
};
use super::{
    ChatInput, McpRequestInput, McpResponseInput, PostCallInput, PostInspectionInput, PreCallInput,
};
use crate::helpers::HelperResult;

pub(super) fn construct(settings: &Value) -> HelperResult {
    let configured = configure(settings)?;
    Ok(json!({
        "guardrail_name": configured.guardrail_name,
        "api_base": configured.api_base,
        "inspect_path": configured.inspect_path,
        "inspection_type": match configured.inspection_type {
            betterlite_config::sections::cisco_verdict_contracts::InspectionType::Chat => "chat",
            betterlite_config::sections::cisco_verdict_contracts::InspectionType::Mcp => "mcp",
        },
        "on_flagged_action": match configured.on_flagged_action {
            betterlite_config::sections::cisco_verdict_contracts::FlaggedAction::Block => "block",
            betterlite_config::sections::cisco_verdict_contracts::FlaggedAction::Monitor => "monitor",
        },
        "fallback_on_error": match configured.fallback_on_error {
            betterlite_config::sections::cisco_verdict_contracts::FallbackAction::Allow => "allow",
            betterlite_config::sections::cisco_verdict_contracts::FallbackAction::Block => "block",
        },
        "timeout_seconds": configured.timeout,
    }))
}

pub(super) fn headers(settings: &Value) -> HelperResult {
    let guardrail = guardrail(settings)?;
    let map = build_headers(&guardrail)
        .map_err(|error| fail("CiscoAIDefenseGuardrailAPIError", error.0))?;
    let mut rendered = std::collections::BTreeMap::new();
    for (name, value) in &map {
        let text = value
            .to_str()
            .map_err(|error| fail("OSError", error.to_string()))?;
        rendered.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(json!({ "headers": rendered }))
}

pub(super) fn pre_call_hook(args: &PreCallInput, is_moderation: bool) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data: Fields = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let hook_args = HookArgs {
        call_type: args.call_type.as_deref(),
        should_run: args.should_run,
        key_user_id: args.key_user_id.as_deref(),
    };
    let outcome = if is_moderation {
        runtime.block_on(async_moderation_hook(&guardrail, &mut data, hook_args))
    } else {
        runtime.block_on(async_pre_call_hook(&guardrail, &mut data, hook_args))
    };
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome.as_ref().map(outcome_tag),
        "applied_guardrails": applied_guardrails(&data),
    }))
}

pub(super) fn post_call_hook(args: &PostCallInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data: Fields = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let mut response: Fields = serde_json::from_value(args.response.clone()).unwrap_or_default();
    let hook_args = PostCallArgs {
        should_run: args.should_run,
        key_user_id: args.key_user_id.as_deref(),
    };
    let outcome = runtime.block_on(async_post_call_success_hook(
        &guardrail,
        &mut data,
        &mut response,
        hook_args,
    ));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome.as_ref().map(outcome_tag),
        "applied_guardrails": applied_guardrails(&data),
        "response": serde_json::from_str(&serde_json::to_string(&response).unwrap_or_default())
            .unwrap_or(Value::Null),
    }))
}

pub(super) fn chat(args: &ChatInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data: Fields = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let messages = request_messages(&data);
    let direction = match args.direction.as_deref() {
        Some("output") => Direction::Output,
        _ => Direction::Input,
    };
    let chat_args = ChatInspectionArgs {
        messages,
        direction,
        key_user_id: args.key_user_id.as_deref(),
    };
    let outcome = runtime.block_on(inspect_chat(
        &guardrail,
        chat_args,
        &mut data,
        ResponseSurface::None,
    ));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome_tag(&outcome),
    }))
}

// mcp_request and mcp_response share this setup, differing only in which inspect fn they drive.
macro_rules! mcp_op {
    ($fn_name:ident, $input_ty:ty, $run:expr) => {
        pub(super) fn $fn_name(args: &$input_ty) -> HelperResult {
            let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
            let mut guardrail = guardrail(&args.settings)?;
            guardrail.settings.api_base = base;
            let mut data: Fields =
                serde_json::from_value(args.request_data.clone()).unwrap_or_default();
            let outcome = $run(&runtime, &guardrail, &mut data, args);
            drop(silent);
            let (path, body) = recorded_call(mock)?;
            Ok(json!({
                "path": path,
                "request": body,
                "outcome": outcome.as_ref().map(outcome_tag),
            }))
        }
    };
}

mcp_op!(
    mcp_request,
    McpRequestInput,
    |runtime: &tokio::runtime::Runtime, guardrail, data, _args| {
        runtime.block_on(inspect_mcp_request(guardrail, data))
    }
);
mcp_op!(
    mcp_response,
    McpResponseInput,
    |runtime: &tokio::runtime::Runtime, guardrail, data, args: &McpResponseInput| {
        let response_json = Json::parse(&args.response.to_string());
        runtime.block_on(inspect_mcp_response(guardrail, data, &response_json))
    }
);

pub(super) fn post_inspection_op(args: &PostInspectionInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let url = format!(
        "{}{}",
        guardrail.settings.api_base, guardrail.settings.inspect_path
    );
    let result = runtime.block_on(post_inspection(
        &guardrail,
        &url,
        &args.payload,
        &args.surface,
    ));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    let value = result
        .map(|json| json_to_value(&json))
        .map_err(|error| fail("CiscoAIDefenseGuardrailAPIError", error.0))?;
    Ok(json!({
        "path": path,
        "request": body,
        "result": value,
    }))
}
