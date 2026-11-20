#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::api::prompt_hook_dispatch::PromptCheckError;
use crate::api::prompt_hook_dispatch::PromptHookDispatch;
use crate::api::prompt_hook_dispatch::selection::{PromptSelectionRequest, select_prompt_manager};
use crate::api::prompt_hook_dispatch::tests::support::{
    Backend, FakeFactory, FakeManager, HookBehavior, Lists, backend, list, logging_state, object,
    registry, store,
};
use crate::api::vector_store_registry::store::VectorStoreRegistry;

fn fake(name: &'static str, check: Result<bool, PromptCheckError>) -> Arc<FakeManager> {
    Arc::new(FakeManager::new(name, check, HookBehavior::Echo))
}

fn request<'request>(
    model: &'request str,
    non_default_params: &'request JsonObject,
) -> PromptSelectionRequest<'request> {
    PromptSelectionRequest {
        model,
        non_default_params,
        tools: None,
        prompt_id: None,
        prompt_spec: None,
        dynamic_callback_params: None,
    }
}

fn select(
    state: &mut LoggingState,
    lists: &mut Lists,
    factory: &FakeFactory,
    vector_stores: Option<&VectorStoreRegistry>,
    request: &PromptSelectionRequest<'_>,
) -> Option<Backend> {
    let mut dispatch = PromptHookDispatch::new(
        state,
        &mut lists.managers,
        &mut lists.callbacks,
        factory,
        vector_stores,
    );
    select_prompt_manager(&mut dispatch, request)
}

fn integration(state: &LoggingState) -> Option<&Json> {
    state.model_call_details.get("prompt_integration")
}

fn empty_dynamic() -> Json {
    Json::Object(JsonObject::new())
}

#[test]
fn a_model_prefix_selects_the_factory_manager() {
    let selected = fake("dotprompt_manager", Ok(false));
    let manager = backend(&selected);
    let factory = FakeFactory::new(vec!["dotprompt"], vec![("dotprompt", Arc::clone(&manager))]);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = JsonObject::new();
    let request = request("dotprompt/prod/prompt", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert_eq!(factory.inits(), ["dotprompt".to_owned()]);
    assert_eq!(
        integration(&state),
        Some(&Json::Text("dotprompt".to_owned()))
    );
    assert!(lists.managers.is_empty());
    assert!(lists.callbacks.is_empty());
}

#[test]
fn a_factory_name_without_a_manager_falls_through_to_the_next_name() {
    let selected = fake("dotprompt_manager", Ok(false));
    let manager = backend(&selected);
    let factory = FakeFactory::new(
        vec!["dot", "dotprompt"],
        vec![("dotprompt", Arc::clone(&manager))],
    );
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = JsonObject::new();
    let request = request("dotprompt/prod/prompt", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert_eq!(factory.inits(), ["dot".to_owned(), "dotprompt".to_owned()]);
}

#[test]
fn a_factory_name_without_a_manager_falls_through_to_auto_detection() {
    let detected = fake("gitlab_manager", Ok(true));
    let manager = backend(&detected);
    let factory = FakeFactory::new(vec!["dotprompt"], Vec::new());
    let mut state = logging_state();
    let mut lists = Lists {
        managers: vec![Arc::clone(&manager)],
        callbacks: Vec::new(),
    };
    let params = JsonObject::new();
    let dynamic = empty_dynamic();
    let mut request = request("dotprompt/prod/prompt", &params);
    request.prompt_id = Some("prompt-1");
    request.dynamic_callback_params = Some(&dynamic);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert_eq!(factory.inits(), ["dotprompt".to_owned()]);
    assert_eq!(
        integration(&state),
        Some(&Json::Text("gitlab_manager".to_owned()))
    );
}

#[test]
fn a_factory_manager_wins_over_auto_detection() {
    let selected = fake("factory_manager", Ok(false));
    let detected = fake("detected_manager", Ok(true));
    let manager = backend(&selected);
    let factory = FakeFactory::new(vec!["dotprompt"], vec![("dotprompt", Arc::clone(&manager))]);
    let mut state = logging_state();
    let mut lists = Lists {
        managers: vec![backend(&detected)],
        callbacks: Vec::new(),
    };
    let params = JsonObject::new();
    let dynamic = empty_dynamic();
    let mut request = request("dotprompt/prod/prompt", &params);
    request.prompt_id = Some("prompt-1");
    request.dynamic_callback_params = Some(&dynamic);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert!(detected.checks().is_empty());
    assert_eq!(
        integration(&state),
        Some(&Json::Text("dotprompt".to_owned()))
    );
}

#[test]
fn auto_detection_returns_the_first_manager_that_runs() {
    let first = fake("first_manager", Ok(false));
    let second = fake("second_manager", Ok(true));
    let mut lists = Lists {
        managers: vec![backend(&first), backend(&second)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let dynamic = empty_dynamic();
    let mut request = request("gpt-4", &params);
    request.prompt_id = Some("prompt-1");
    request.dynamic_callback_params = Some(&dynamic);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &backend(&second)));
    assert_eq!(first.checks(), [Some("prompt-1".to_owned())]);
    assert_eq!(second.checks(), [Some("prompt-1".to_owned())]);
    assert_eq!(
        integration(&state),
        Some(&Json::Text("second_manager".to_owned()))
    );
}

#[test]
fn auto_detection_skips_a_manager_whose_check_errors() {
    let broken = fake("broken_manager", Err(PromptCheckError));
    let second = fake("second_manager", Ok(true));
    let mut lists = Lists {
        managers: vec![backend(&broken), backend(&second)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let dynamic = empty_dynamic();
    let mut request = request("gpt-4", &params);
    request.prompt_id = Some("prompt-1");
    request.dynamic_callback_params = Some(&dynamic);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &backend(&second)));
    assert_eq!(broken.checks(), [Some("prompt-1".to_owned())]);
}

#[test]
fn auto_detection_needs_dynamic_callback_params() {
    let first = fake("first_manager", Ok(false));
    let second = fake("second_manager", Ok(true));
    let manager = backend(&first);
    let mut lists = Lists {
        managers: vec![Arc::clone(&manager), backend(&second)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let mut request = request("gpt-4", &params);
    request.prompt_id = Some("prompt-1");
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert!(first.checks().is_empty());
    assert!(second.checks().is_empty());
}

#[test]
fn an_empty_prompt_id_skips_auto_detection() {
    let first = fake("first_manager", Ok(false));
    let manager = backend(&first);
    let mut lists = Lists {
        managers: vec![Arc::clone(&manager)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let dynamic = empty_dynamic();
    let mut request = request("gpt-4", &params);
    request.prompt_id = Some("");
    request.dynamic_callback_params = Some(&dynamic);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert!(first.checks().is_empty());
}

#[test]
fn the_fallback_skips_a_manager_that_does_not_run_without_a_prompt_id() {
    let first = fake("first_manager", Ok(false));
    let second = fake("second_manager", Ok(true));
    let mut lists = Lists {
        managers: vec![backend(&first), backend(&second)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &backend(&second)));
    assert_eq!(first.checks(), [None]);
    assert_eq!(second.checks(), [None]);
    assert_eq!(
        integration(&state),
        Some(&Json::Text("second_manager".to_owned()))
    );
}

#[test]
fn the_fallback_counts_a_check_error_as_no() {
    let broken = fake("broken_manager", Err(PromptCheckError));
    let declining = fake("declining_manager", Ok(false));
    let mut lists = Lists {
        managers: vec![backend(&broken), backend(&declining)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(found.is_none());
    assert_eq!(broken.checks(), [None]);
    assert_eq!(declining.checks(), [None]);
    assert!(integration(&state).is_none());
}

#[test]
fn the_fallback_does_not_check_a_manager_when_a_prompt_id_is_set() {
    let first = fake("first_manager", Ok(false));
    let manager = backend(&first);
    let mut lists = Lists {
        managers: vec![Arc::clone(&manager)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = JsonObject::new();
    let dynamic = empty_dynamic();
    let mut request = request("gpt-4", &params);
    request.prompt_id = Some("prompt-1");
    request.dynamic_callback_params = Some(&dynamic);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert_eq!(first.checks(), [Some("prompt-1".to_owned())]);
}

#[test]
fn the_cache_control_hook_runs_with_injection_points() {
    let hook = fake("anthropic_cache_control_hook", Ok(false));
    let manager = backend(&hook);
    let factory = FakeFactory::new(
        Vec::new(),
        vec![("anthropic_cache_control_hook", Arc::clone(&manager))],
    );
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = object(vec![("cache_control_injection_points", list(&["system"]))]);
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert_eq!(factory.inits(), ["anthropic_cache_control_hook".to_owned()]);
    assert_eq!(
        integration(&state),
        Some(&Json::Text("anthropic_cache_control_hook".to_owned()))
    );
}

#[test]
fn a_registered_manager_wins_over_the_cache_control_hook() {
    let registered = fake("registered_manager", Ok(true));
    let manager = backend(&registered);
    let mut lists = Lists {
        managers: vec![Arc::clone(&manager)],
        callbacks: Vec::new(),
    };
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let mut state = logging_state();
    let params = object(vec![("cache_control_injection_points", list(&["system"]))]);
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &manager));
    assert!(factory.inits().is_empty());
}

#[test]
fn the_cache_control_hook_wins_over_the_vector_store_hook() {
    let cache = fake("anthropic_cache_control_hook", Ok(false));
    let vector = fake("vector_store_pre_call_hook", Ok(false));
    let cache_manager = backend(&cache);
    let vector_manager = backend(&vector);
    let factory = FakeFactory::new(
        Vec::new(),
        vec![
            ("anthropic_cache_control_hook", Arc::clone(&cache_manager)),
            ("vector_store_pre_call_hook", Arc::clone(&vector_manager)),
        ],
    );
    let stores = registry(vec![store("vs-1")]);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = object(vec![("cache_control_injection_points", list(&["system"]))]);
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, Some(&stores), &request);
    assert!(Arc::ptr_eq(found.as_ref().unwrap(), &cache_manager));
    assert_eq!(factory.inits(), ["anthropic_cache_control_hook".to_owned()]);
    assert!(lists.callbacks.is_empty());
}

#[test]
fn the_vector_store_hook_is_registered_once() {
    let hook = fake("vector_store_pre_call_hook", Ok(false));
    let manager = backend(&hook);
    let factory = FakeFactory::new(
        Vec::new(),
        vec![("vector_store_pre_call_hook", Arc::clone(&manager))],
    );
    let stores = registry(vec![store("vs-1")]);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = JsonObject::new();
    let request = request("gpt-4", &params);
    let first = select(&mut state, &mut lists, &factory, Some(&stores), &request);
    assert!(Arc::ptr_eq(first.as_ref().unwrap(), &manager));
    assert_eq!(lists.callbacks.len(), 1);
    let second = select(&mut state, &mut lists, &factory, Some(&stores), &request);
    assert!(Arc::ptr_eq(second.as_ref().unwrap(), &manager));
    assert_eq!(lists.callbacks.len(), 1);
    assert!(lists.managers.is_empty());
    assert!(hook.checks().is_empty());
    assert_eq!(
        factory.inits(),
        [
            "vector_store_pre_call_hook".to_owned(),
            "vector_store_pre_call_hook".to_owned()
        ]
    );
    assert_eq!(
        integration(&state),
        Some(&Json::Text("vector_store_pre_call_hook".to_owned()))
    );
}

#[test]
fn a_configured_registry_without_a_factory_hook_selects_nothing() {
    let factory = FakeFactory::new(Vec::new(), Vec::new());
    let stores = registry(vec![store("vs-1")]);
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = JsonObject::new();
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, Some(&stores), &request);
    assert!(found.is_none());
    assert_eq!(factory.inits(), ["vector_store_pre_call_hook".to_owned()]);
    assert!(lists.callbacks.is_empty());
    assert!(integration(&state).is_none());
}

#[test]
fn the_vector_store_hook_is_skipped_without_a_registry() {
    let hook = fake("vector_store_pre_call_hook", Ok(false));
    let manager = backend(&hook);
    let factory = FakeFactory::new(
        Vec::new(),
        vec![("vector_store_pre_call_hook", Arc::clone(&manager))],
    );
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = JsonObject::new();
    let request = request("gpt-4", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(found.is_none());
    assert!(factory.inits().is_empty());
    assert!(lists.callbacks.is_empty());
}

#[test]
fn a_model_prefix_is_tested_plainly() {
    let factory = FakeFactory::new(vec!["dotprompt"], Vec::new());
    let mut state = logging_state();
    let mut lists = Lists::default();
    let params = JsonObject::new();
    let request = request("x-dotprompt/model", &params);
    let found = select(&mut state, &mut lists, &factory, None, &request);
    assert!(found.is_none());
    assert!(factory.inits().is_empty());
}
