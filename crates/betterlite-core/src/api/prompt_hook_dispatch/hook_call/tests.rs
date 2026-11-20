#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::orchestration::prompt_orchestration::{
    ChatCompletionPrompt, PromptOrchestrationError,
};

use crate::api::prompt_hook_dispatch::PromptHookDispatch;
use crate::api::prompt_hook_dispatch::hook_call::{
    AsyncPromptCallRequest, PromptCallRequest, async_completion_prompt, get_chat_completion_prompt,
};
use crate::api::prompt_hook_dispatch::tests::support::{
    FakeFactory, FakeManager, HookBehavior, Lists, TestError, backend, json, list, logging_state,
    object, registry, store, text,
};
use crate::api::vector_store_registry::store::VectorStoreRegistry;

fn request<'request>(
    model: &'request str,
    messages: &'request [Json],
    non_default_params: &'request JsonObject,
) -> PromptCallRequest<'request, TestError> {
    PromptCallRequest {
        model,
        messages,
        non_default_params,
        prompt_variables: None,
        prompt_id: None,
        prompt_spec: None,
        manager: None,
        prompt_label: None,
        prompt_version: None,
        request_kwargs: None,
        injected_for_every_deployment: false,
    }
}

async fn call(
    state: &mut LoggingState,
    lists: &mut Lists,
    factory: &FakeFactory,
    vector_stores: Option<&VectorStoreRegistry>,
    request: PromptCallRequest<'_, TestError>,
) -> Result<ChatCompletionPrompt, PromptOrchestrationError<TestError>> {
    let mut dispatch = PromptHookDispatch::new(
        state,
        &mut lists.managers,
        &mut lists.callbacks,
        factory,
        vector_stores,
    );
    get_chat_completion_prompt(&mut dispatch, request).await
}

fn kwargs(deployment: &str) -> JsonObject {
    object(vec![
        ("litellm_metadata", Json::Object(JsonObject::new())),
        (
            "model_info",
            Json::Object(object(vec![("id", text(deployment))])),
        ),
    ])
}

#[tokio::test]
async fn the_sync_hook_receives_the_call_values() {
    let fake = Arc::new(FakeManager::new("manager", Ok(false), HookBehavior::Echo));
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = object(vec![("temperature", json("0.5"))]);
    let variables = object(vec![("name", text("world"))]);
    state
        .callback_dynamic_params
        .insert("user_id".to_owned(), text("u-1"));
    let dynamic = Json::Object(state.callback_dynamic_params.clone());
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(backend(&fake));
    request.prompt_variables = Some(&variables);
    request.prompt_id = Some("prompt-1");
    request.prompt_label = Some("prod");
    request.prompt_version = Some(3);
    let result = call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    let entry = &calls[0];
    assert_eq!(entry.model, "gpt-4");
    assert_eq!(entry.messages, messages);
    assert_eq!(entry.non_default_params, params);
    assert_eq!(entry.prompt_id.as_deref(), Some("prompt-1"));
    assert_eq!(entry.prompt_variables.as_ref(), Some(&variables));
    assert_eq!(entry.dynamic_callback_params, dynamic);
    assert_eq!(entry.prompt_label.as_deref(), Some("prod"));
    assert_eq!(entry.prompt_version, Some(3));
    assert!(!entry.asynchronous);
    assert_eq!(result.messages, messages);
    assert_eq!(result.non_default_params, params);
    assert_eq!(state.messages.as_ref(), Some(&messages));
    assert!(state.model_call_details.get("prompt_integration").is_none());
}

#[tokio::test]
async fn the_async_hook_receives_the_call_values() {
    let fake = Arc::new(FakeManager::new(
        "explicit_manager",
        Ok(false),
        HookBehavior::Echo,
    ));
    let manager = backend(&fake);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&manager));
    request.prompt_id = Some("prompt-1");
    let async_request = AsyncPromptCallRequest {
        call: request,
        tools: None,
    };
    let mut dispatch = PromptHookDispatch::new(
        &mut state,
        &mut lists.managers,
        &mut lists.callbacks,
        &factory,
        None,
    );
    let result = async_completion_prompt(&mut dispatch, async_request)
        .await
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].asynchronous);
    assert_eq!(calls[0].prompt_id.as_deref(), Some("prompt-1"));
    assert_eq!(result.messages, messages);
    assert_eq!(state.messages.as_ref(), Some(&messages));
}

#[tokio::test]
async fn the_explicit_manager_wins_over_selection() {
    let explicit = Arc::new(FakeManager::new(
        "explicit_manager",
        Ok(false),
        HookBehavior::Echo,
    ));
    let factory_manager = Arc::new(FakeManager::new(
        "factory_manager",
        Ok(false),
        HookBehavior::Echo,
    ));
    let explicit_backend = backend(&explicit);
    let factory_backend = backend(&factory_manager);
    let factory = FakeFactory::new(vec!["gpt-4"], vec![("gpt-4", Arc::clone(&factory_backend))]);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&explicit_backend));
    let result = call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    assert_eq!(explicit.calls().len(), 1);
    assert!(factory_manager.calls().is_empty());
    assert!(factory.inits().is_empty());
    assert!(state.model_call_details.get("prompt_integration").is_none());
    assert_eq!(result.messages, messages);
}

#[tokio::test]
async fn empty_params_reach_the_manager_as_an_empty_map() {
    let fake = Arc::new(FakeManager::new(
        "explicit_manager",
        Ok(false),
        HookBehavior::Echo,
    ));
    let manager = backend(&fake);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&manager));
    let result = call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    assert!(fake.calls()[0].non_default_params.is_empty());
    assert!(result.non_default_params.is_empty());
}

#[tokio::test]
async fn a_hook_breakpoint_is_recorded_in_the_request_kwargs() {
    let fake = Arc::new(FakeManager::new(
        "breakpoint_manager",
        Ok(false),
        HookBehavior::AddBreakpoint,
    ));
    let manager = backend(&fake);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut marked = kwargs("dep-1");
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&manager));
    request.request_kwargs = Some(&mut marked);
    let result = call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    assert_eq!(result.messages.len(), 2);
    assert_eq!(state.messages.as_ref(), Some(&result.messages));
    let expected = object(vec![("litellm_gateway_injected_cache", text("dep-1"))]);
    assert_eq!(
        marked.get("litellm_metadata"),
        Some(&Json::Object(expected))
    );
}

#[tokio::test]
async fn a_hook_without_new_breakpoints_leaves_the_request_kwargs_untouched() {
    let fake = Arc::new(FakeManager::new(
        "echo_manager",
        Ok(false),
        HookBehavior::Echo,
    ));
    let manager = backend(&fake);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut marked = kwargs("dep-1");
    let before = marked.clone();
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&manager));
    request.request_kwargs = Some(&mut marked);
    call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    assert_eq!(marked, before);
}

#[tokio::test]
async fn the_every_deployment_flag_marks_the_empty_sentinel() {
    let fake = Arc::new(FakeManager::new(
        "breakpoint_manager",
        Ok(false),
        HookBehavior::AddBreakpoint,
    ));
    let manager = backend(&fake);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut marked = kwargs("dep-1");
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&manager));
    request.request_kwargs = Some(&mut marked);
    request.injected_for_every_deployment = true;
    call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    let expected = object(vec![("litellm_gateway_injected_cache", text(""))]);
    assert_eq!(
        marked.get("litellm_metadata"),
        Some(&Json::Object(expected))
    );
}

#[tokio::test]
async fn messages_are_stored_without_a_manager() {
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = object(vec![("temperature", json("0.5"))]);
    let request = request("gpt-4", messages.as_slice(), &params);
    let result = call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    assert_eq!(result.model, "gpt-4");
    assert_eq!(result.messages, messages);
    assert_eq!(result.non_default_params, params);
    assert_eq!(state.messages.as_ref(), Some(&messages));
    assert!(state.model_call_details.get("prompt_integration").is_none());
}

#[tokio::test]
async fn a_request_without_a_manager_leaves_the_request_kwargs_untouched() {
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(
        r#"{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}"#,
    )];
    let params = JsonObject::new();
    let mut marked = kwargs("dep-1");
    let before = marked.clone();
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.request_kwargs = Some(&mut marked);
    call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap();
    assert_eq!(marked, before);
    assert_eq!(state.messages.as_ref(), Some(&messages));
}

#[tokio::test]
async fn a_manager_error_propagates_unchanged() {
    let fake = Arc::new(FakeManager::new(
        "failing_manager",
        Ok(false),
        HookBehavior::Fail,
    ));
    let manager = backend(&fake);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let mut request = request("gpt-4", messages.as_slice(), &params);
    request.manager = Some(Arc::clone(&manager));
    request.prompt_id = Some("prompt-1");
    let error = call(&mut state, &mut lists, &factory, None, request)
        .await
        .unwrap_err();
    match error {
        PromptOrchestrationError::Compile { prompt_id, source } => {
            assert_eq!(prompt_id, "prompt-1");
            assert_eq!(source, TestError::Failed);
        }
    }
    assert!(state.messages.is_none());
}

#[tokio::test]
async fn the_async_call_selects_through_the_vector_store_hook() {
    let hook = Arc::new(FakeManager::new(
        "vector_store_pre_call_hook",
        Ok(false),
        HookBehavior::Echo,
    ));
    let manager = backend(&hook);
    let factory = FakeFactory::new(
        Vec::new(),
        vec![("vector_store_pre_call_hook", Arc::clone(&manager))],
    );
    let stores = registry(vec![store("vs-1")]);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let messages = vec![json(r#"{"role":"user","content":"hi"}"#)];
    let params = JsonObject::new();
    let tools = vec![object(vec![
        ("type", text("file_search")),
        ("vector_store_ids", list(&["vs-1"])),
    ])];
    let request = request("gpt-4", messages.as_slice(), &params);
    let async_request = AsyncPromptCallRequest {
        call: request,
        tools: Some(tools.as_slice()),
    };
    let mut dispatch = PromptHookDispatch::new(
        &mut state,
        &mut lists.managers,
        &mut lists.callbacks,
        &factory,
        Some(&stores),
    );
    let result = async_completion_prompt(&mut dispatch, async_request)
        .await
        .unwrap();
    assert_eq!(hook.calls().len(), 1);
    assert_eq!(factory.inits(), ["vector_store_pre_call_hook".to_owned()]);
    assert_eq!(lists.callbacks.len(), 1);
    assert_eq!(result.messages, messages);
}
