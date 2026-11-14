#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::prompt_dispatch::factory::{PromptFactoryError, prompt_factory};
use crate::prompt_dispatch::identity::{PromptIdentity, resolve_prompt_identity};
use crate::prompt_dispatch::known_callbacks::KNOWN_CALLBACKS;
use crate::prompt_dispatch::merge::{PromptCall, merge_prompt_request};
use crate::prompt_dispatch::ports::{CompilePromptCall, CompiledPrompt, PromptPorts};
use crate::prompt_dispatch::predicate::is_prompt_model;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn object(pairs: &[(&str, Json)]) -> JsonObject {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn json_object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(object(pairs))
}

fn deployment_with(model: Json) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), model);
    DeploymentSpec {
        model_name: "group".to_owned(),
        litellm_params,
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

fn deployment(model: &str) -> DeploymentSpec {
    deployment_with(text(model))
}

fn compiled() -> CompiledPrompt {
    CompiledPrompt {
        model: "gpt-4o".to_owned(),
        messages: vec![json_object(&[
            ("role", text("user")),
            ("content", text("compiled")),
        ])],
        optional_params: object(&[("temperature", number(1))]),
    }
}

fn identity() -> PromptIdentity {
    PromptIdentity {
        prompt_id: Some(text("request-id")),
        prompt_variables: Some(json_object(&[("name", text("world"))])),
        prompt_label: Some(text("production")),
    }
}

#[rstest]
#[case::known_prefix("langfuse/prompt", true)]
#[case::extra_slash("langfuse/a/b", true)]
#[case::no_slash("langfuse", false)]
#[case::unknown_prefix("mystore/prompt", false)]
#[case::empty_prefix("/prompt", false)]
fn is_prompt_model_reads_the_text_before_the_first_slash(
    #[case] model: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        is_prompt_model(&[deployment(model)], &KNOWN_CALLBACKS),
        expected
    );
}

#[test]
fn is_prompt_model_is_false_for_two_deployments() {
    let deployments = [deployment("langfuse/prompt"), deployment("langfuse/other")];
    assert!(!is_prompt_model(&deployments, &KNOWN_CALLBACKS));
}

#[test]
fn is_prompt_model_is_false_for_no_deployments() {
    assert!(!is_prompt_model(&[], &KNOWN_CALLBACKS));
}

#[test]
fn is_prompt_model_is_false_when_the_deployment_has_no_model() {
    let mut absent = deployment("langfuse/prompt");
    absent.litellm_params.shift_remove("model");
    assert!(!is_prompt_model(&[absent], &KNOWN_CALLBACKS));
    assert!(!is_prompt_model(
        &[deployment_with(Json::Null)],
        &KNOWN_CALLBACKS
    ));
}

#[test]
fn is_prompt_model_is_false_when_the_model_is_not_a_string() {
    assert!(!is_prompt_model(
        &[deployment_with(number(7))],
        &KNOWN_CALLBACKS
    ));
}

#[test]
fn is_prompt_model_reads_the_callback_list_it_is_given() {
    assert!(is_prompt_model(
        &[deployment("mystore/prompt")],
        &["mystore"]
    ));
    assert!(!is_prompt_model(
        &[deployment("mystore/prompt")],
        &["langfuse"]
    ));
}

#[test]
fn resolve_prompt_identity_prefers_set_request_values() {
    let request = object(&[
        ("prompt_id", text("request-id")),
        (
            "prompt_variables",
            json_object(&[("name", text("request"))]),
        ),
        ("prompt_label", text("request-label")),
    ]);
    let data = object(&[
        ("prompt_id", text("deployment-id")),
        (
            "prompt_variables",
            json_object(&[("name", text("deployment"))]),
        ),
        ("prompt_label", text("deployment-label")),
    ]);
    let resolved = resolve_prompt_identity(&request, &data).unwrap();
    assert_eq!(resolved.prompt_id, Some(text("request-id")));
    assert_eq!(
        resolved.prompt_variables,
        Some(json_object(&[("name", text("request"))]))
    );
    assert_eq!(resolved.prompt_label, Some(text("request-label")));
}

#[test]
fn resolve_prompt_identity_falls_back_when_the_request_value_is_null() {
    let request = object(&[
        ("prompt_id", Json::Null),
        ("prompt_variables", Json::Null),
        ("prompt_label", Json::Null),
    ]);
    let data = object(&[
        ("prompt_id", text("deployment-id")),
        (
            "prompt_variables",
            json_object(&[("name", text("deployment"))]),
        ),
        ("prompt_label", text("deployment-label")),
    ]);
    let resolved = resolve_prompt_identity(&request, &data).unwrap();
    assert_eq!(resolved.prompt_id, Some(text("deployment-id")));
    assert_eq!(
        resolved.prompt_variables,
        Some(json_object(&[("name", text("deployment"))]))
    );
    assert_eq!(resolved.prompt_label, Some(text("deployment-label")));
}

#[test]
fn resolve_prompt_identity_keeps_an_empty_request_value() {
    let request = object(&[
        ("prompt_id", text("")),
        ("prompt_variables", json_object(&[])),
    ]);
    let data = object(&[
        ("prompt_id", text("deployment-id")),
        (
            "prompt_variables",
            json_object(&[("name", text("deployment"))]),
        ),
    ]);
    let resolved = resolve_prompt_identity(&request, &data).unwrap();
    assert_eq!(resolved.prompt_id, Some(text("")));
    assert_eq!(resolved.prompt_variables, Some(json_object(&[])));
}

#[test]
fn resolve_prompt_identity_keeps_an_empty_deployment_value() {
    let data = object(&[("prompt_id", text(""))]);
    let resolved = resolve_prompt_identity(&JsonObject::new(), &data).unwrap();
    assert_eq!(resolved.prompt_id, Some(text("")));
}

#[test]
fn resolve_prompt_identity_skips_the_prompt_id_check_for_litellm_agent_models() {
    let data = object(&[("model", text("litellm_agent/some-agent"))]);
    let resolved = resolve_prompt_identity(&JsonObject::new(), &data).unwrap();
    assert_eq!(resolved.prompt_id, None);
}

#[test]
fn resolve_prompt_identity_passes_a_non_string_prompt_id_for_litellm_agent_models() {
    let request = object(&[("prompt_id", number(7))]);
    let data = object(&[("model", text("litellm_agent/some-agent"))]);
    let resolved = resolve_prompt_identity(&request, &data).unwrap();
    assert_eq!(resolved.prompt_id, Some(number(7)));
}

#[rstest]
#[case::missing_prompt_id(object(&[]), "prompt_id must be a string")]
#[case::number_prompt_id(object(&[("prompt_id", number(7))]), "prompt_id must be a string")]
#[case::list_prompt_variables(
    object(&[("prompt_id", text("greeting")), ("prompt_variables", Json::List(vec![number(1)]))]),
    "prompt_variables must be an object"
)]
#[case::text_prompt_variables(
    object(&[("prompt_id", text("greeting")), ("prompt_variables", text("x"))]),
    "prompt_variables must be an object"
)]
fn resolve_prompt_identity_rejects_the_wrong_types(
    #[case] request: JsonObject,
    #[case] message: &str,
) {
    let error = resolve_prompt_identity(&request, &JsonObject::new()).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn merge_prompt_request_drops_the_six_prompt_keys_from_the_deployment_params() {
    let data = object(&[
        (
            "bitbucket_config",
            json_object(&[("repository", text("repo"))]),
        ),
        ("dotprompt_config", json_object(&[("path", text("p"))])),
        ("prompt_id", text("deployment-id")),
        (
            "prompt_variables",
            json_object(&[("name", text("deployment"))]),
        ),
        ("prompt_label", text("deployment-label")),
        ("prompt_version", number(2)),
        ("api_base", text("https://api.example")),
    ]);
    let merged = merge_prompt_request(&data, &JsonObject::new(), &compiled(), &identity(), true);
    assert!(!merged.fields.contains_key("bitbucket_config"));
    assert!(!merged.fields.contains_key("dotprompt_config"));
    assert!(!merged.fields.contains_key("prompt_version"));
    assert_eq!(merged.fields.get("prompt_id"), Some(&text("request-id")));
    assert_eq!(
        merged.fields.get("api_base"),
        Some(&text("https://api.example"))
    );
    assert_eq!(merged.call, PromptCall::WithFallbacks);
}

#[test]
fn merge_prompt_request_lets_the_request_win_and_then_the_optional_params() {
    let data = object(&[
        ("api_base", text("deployment")),
        ("timeout", number(1)),
        ("seed", number(1)),
    ]);
    let request = object(&[("api_base", text("request")), ("timeout", number(2))]);
    let compiled = CompiledPrompt {
        model: "gpt-4o".to_owned(),
        messages: Vec::new(),
        optional_params: object(&[("timeout", number(3))]),
    };
    let merged = merge_prompt_request(&data, &request, &compiled, &identity(), true);
    assert_eq!(merged.fields.get("api_base"), Some(&text("request")));
    assert_eq!(merged.fields.get("timeout"), Some(&number(3)));
    assert_eq!(merged.fields.get("seed"), Some(&number(1)));
}

#[test]
fn merge_prompt_request_stamps_the_compiled_fields_and_the_identity() {
    let request = object(&[
        ("litellm_logging_obj", text("handle")),
        ("prompt_id", text("request-id")),
        ("prompt_variables", json_object(&[("name", text("world"))])),
        ("prompt_label", text("production")),
    ]);
    let compiled = CompiledPrompt {
        model: "gpt-4o".to_owned(),
        messages: vec![json_object(&[("role", text("user"))])],
        optional_params: object(&[("litellm_logging_obj", text("replaced"))]),
    };
    let merged = merge_prompt_request(&JsonObject::new(), &request, &compiled, &identity(), true);
    assert_eq!(merged.fields.get("model"), Some(&text("gpt-4o")));
    assert_eq!(
        merged.fields.get("messages"),
        Some(&Json::List(compiled.messages.clone()))
    );
    assert_eq!(
        merged.fields.get("litellm_logging_obj"),
        Some(&text("handle"))
    );
    assert_eq!(merged.fields.get("prompt_id"), Some(&text("request-id")));
    assert_eq!(
        merged.fields.get("prompt_variables"),
        Some(&json_object(&[("name", text("world"))]))
    );
    assert_eq!(merged.fields.get("prompt_label"), Some(&text("production")));
}

#[test]
fn merge_prompt_request_writes_null_for_an_absent_identity_value() {
    let merged = merge_prompt_request(
        &JsonObject::new(),
        &JsonObject::new(),
        &compiled(),
        &PromptIdentity {
            prompt_id: Some(text("greeting")),
            prompt_variables: None,
            prompt_label: None,
        },
        true,
    );
    assert_eq!(merged.fields.get("prompt_variables"), Some(&Json::Null));
    assert_eq!(merged.fields.get("prompt_label"), Some(&Json::Null));
}

#[test]
fn merge_prompt_request_drops_original_function_for_a_model_without_deployments() {
    let request = object(&[("original_function", text("acompletion"))]);
    let merged = merge_prompt_request(
        &JsonObject::new(),
        &request,
        &compiled(),
        &identity(),
        false,
    );
    assert_eq!(merged.call, PromptCall::Direct);
    assert!(!merged.fields.contains_key("original_function"));
}

#[test]
fn merge_prompt_request_keeps_original_function_for_a_model_group() {
    let request = object(&[("original_function", text("acompletion"))]);
    let merged = merge_prompt_request(&JsonObject::new(), &request, &compiled(), &identity(), true);
    assert_eq!(merged.call, PromptCall::WithFallbacks);
    assert_eq!(
        merged.fields.get("original_function"),
        Some(&text("acompletion"))
    );
}

#[derive(Default)]
struct Observation {
    selected: Option<(String, Option<Json>)>,
    updated: bool,
    compile_model: Option<String>,
    compile_messages: Option<Vec<Json>>,
    compile_non_default: Option<JsonObject>,
    compile_prompt_id: Option<Option<Json>>,
    compile_variables: Option<Option<Json>>,
    compile_label: Option<Option<Json>>,
    compile_injected: Option<bool>,
    direct: Option<JsonObject>,
    fallbacks: Option<JsonObject>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake port fails")]
struct FakeFailure;

struct FakePorts {
    deployment: DeploymentSpec,
    compiled: CompiledPrompt,
    group_model: String,
    observed: Mutex<Observation>,
}

#[async_trait]
impl PromptPorts for FakePorts {
    type Error = FakeFailure;
    type Response = Json;

    async fn select_deployment(
        &self,
        model: &str,
        _messages: &[Json],
        specific_deployment: Option<&Json>,
        _request: &JsonObject,
    ) -> Result<DeploymentSpec, FakeFailure> {
        self.observed.lock().unwrap().selected =
            Some((model.to_owned(), specific_deployment.cloned()));
        Ok(self.deployment.clone())
    }

    fn update_with_deployment(&self, request: &mut JsonObject, _deployment: &DeploymentSpec) {
        self.observed.lock().unwrap().updated = true;
        request.insert("deployment_marker".to_owned(), Json::Bool(true));
    }

    async fn compile_prompt(
        &self,
        call: CompilePromptCall<'_>,
    ) -> Result<CompiledPrompt, FakeFailure> {
        let mut observed = self.observed.lock().unwrap();
        observed.compile_model = Some(call.model.to_owned());
        observed.compile_messages = Some(call.messages.to_vec());
        observed.compile_non_default = Some(call.non_default_params.clone());
        observed.compile_prompt_id = Some(call.prompt_id.cloned());
        observed.compile_variables = Some(call.prompt_variables.cloned());
        observed.compile_label = Some(call.prompt_label.cloned());
        observed.compile_injected = Some(call.injected_for_every_deployment);
        drop(observed);
        Ok(self.compiled.clone())
    }

    fn has_deployments(&self, model: &str) -> bool {
        model == self.group_model
    }

    async fn call_direct(&self, request: JsonObject) -> Result<Json, FakeFailure> {
        self.observed.lock().unwrap().direct = Some(request);
        Ok(text("answer"))
    }

    async fn call_with_fallbacks(&self, request: JsonObject) -> Result<Json, FakeFailure> {
        self.observed.lock().unwrap().fallbacks = Some(request);
        Ok(text("answer"))
    }
}

fn prompt_deployment() -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), text("langfuse/my-prompt"));
    litellm_params.insert("prompt_id".to_owned(), text("greeting"));
    litellm_params.insert(
        "prompt_variables".to_owned(),
        json_object(&[("name", text("world"))]),
    );
    litellm_params.insert("prompt_label".to_owned(), text("production"));
    DeploymentSpec {
        model_name: "support-prompt".to_owned(),
        litellm_params,
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

#[tokio::test]
async fn prompt_factory_passes_the_reference_arguments_to_every_port() {
    let ports = FakePorts {
        deployment: prompt_deployment(),
        compiled: compiled(),
        group_model: "other-model".to_owned(),
        observed: Mutex::new(Observation::default()),
    };
    let request = object(&[
        ("specific_deployment", Json::Bool(true)),
        ("litellm_logging_obj", text("handle")),
        ("custom_provider_param", text("kept")),
        ("temperature", number(1)),
        ("original_function", text("acompletion")),
    ]);
    let messages = [json_object(&[("role", text("user"))])];
    let answer = prompt_factory("support-prompt", &messages, request, &ports)
        .await
        .unwrap();
    assert_eq!(answer, text("answer"));
    assert_reference_arguments(&ports.observed.lock().unwrap());
}

fn assert_reference_arguments(observed: &Observation) {
    let (selected_model, specific) = observed.selected.as_ref().unwrap();
    assert_eq!(selected_model, "support-prompt");
    assert_eq!(specific, &Some(Json::Bool(true)));
    assert!(observed.updated);
    assert_eq!(
        observed.compile_model.as_deref(),
        Some("langfuse/my-prompt")
    );
    assert_eq!(
        observed.compile_messages,
        Some(vec![json_object(&[("role", text("user"))])])
    );
    assert_eq!(
        observed.compile_variables,
        Some(Some(json_object(&[("name", text("world"))])))
    );
    assert_eq!(observed.compile_label, Some(Some(text("production"))));
    assert_eq!(
        observed.compile_non_default,
        Some(object(&[
            ("custom_provider_param", text("kept")),
            ("original_function", text("acompletion")),
            ("deployment_marker", Json::Bool(true)),
        ]))
    );
    assert_eq!(observed.compile_prompt_id, Some(Some(text("greeting"))));
    assert_eq!(observed.compile_injected, Some(true));
    let called = observed.direct.as_ref().unwrap();
    assert!(!called.contains_key("specific_deployment"));
    assert!(!called.contains_key("original_function"));
    assert_eq!(called.get("deployment_marker"), Some(&Json::Bool(true)));
    assert!(observed.fallbacks.is_none());
}

#[tokio::test]
async fn prompt_factory_sends_a_model_group_through_the_fallbacks() {
    let ports = FakePorts {
        deployment: prompt_deployment(),
        compiled: CompiledPrompt {
            model: "gpt-4o".to_owned(),
            messages: Vec::new(),
            optional_params: JsonObject::new(),
        },
        group_model: "gpt-4o".to_owned(),
        observed: Mutex::new(Observation::default()),
    };
    let request = object(&[("original_function", text("acompletion"))]);
    let answer = prompt_factory("support-prompt", &[], request, &ports)
        .await
        .unwrap();
    assert_eq!(answer, text("answer"));
    let observed = ports.observed.lock().unwrap();
    assert!(observed.direct.is_none());
    let called = observed.fallbacks.as_ref().unwrap();
    assert_eq!(called.get("original_function"), Some(&text("acompletion")));
    assert_eq!(called.get("model"), Some(&text("gpt-4o")));
}

#[tokio::test]
async fn prompt_factory_returns_the_identity_error_before_it_compiles() {
    let ports = FakePorts {
        deployment: deployment("langfuse/my-prompt"),
        compiled: compiled(),
        group_model: "gpt-4o".to_owned(),
        observed: Mutex::new(Observation::default()),
    };
    let error = prompt_factory("langfuse/my-prompt", &[], JsonObject::new(), &ports)
        .await
        .unwrap_err();
    assert!(matches!(error, PromptFactoryError::Identity(_)));
    assert!(ports.observed.lock().unwrap().compile_model.is_none());
}

#[tokio::test]
async fn prompt_factory_fails_when_the_deployment_has_no_model() {
    let ports = FakePorts {
        deployment: DeploymentSpec {
            model_name: "group".to_owned(),
            litellm_params: JsonObject::new(),
            model_info: JsonObject::new(),
            extra: JsonObject::new(),
        },
        compiled: compiled(),
        group_model: "gpt-4o".to_owned(),
        observed: Mutex::new(Observation::default()),
    };
    let request = object(&[("prompt_id", text("greeting"))]);
    let error = prompt_factory("group", &[], request, &ports)
        .await
        .unwrap_err();
    assert!(matches!(error, PromptFactoryError::DeploymentModel));
}
