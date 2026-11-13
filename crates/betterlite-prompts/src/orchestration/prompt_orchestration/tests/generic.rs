use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::generic_prompt_api::manager::{GenericPromptManager, GenericPromptSettings};
use betterlite_models::generic_prompt_api::request::{
    PromptApiRequest, PromptApiResponse, PromptApiTransport,
};
use betterlite_models::prompt_contracts::PromptIgnoreFlags;
use betterlite_models::prompt_spec::PromptSpec;
use indexmap::IndexMap;

use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{json, message, params, request};

const BODY: &str = r#"{"prompt_template":[{"role":"system","content":"Be brief"}],"prompt_template_model":"gpt-4o","prompt_template_optional_params":{"temperature":0.5}}"#;
const BODY_WITHOUT_MODEL: &str = r#"{"prompt_template":[{"role":"user","content":"Say hello"}]}"#;

#[derive(Clone)]
struct FakePromptApi {
    replies: Arc<Mutex<VecDeque<Result<PromptApiResponse, io::Error>>>>,
    calls: Arc<Mutex<Vec<PromptApiRequest>>>,
}

impl FakePromptApi {
    fn replying(body: &str) -> Self {
        Self::replying_many(vec![body])
    }

    fn replying_many(bodies: Vec<&str>) -> Self {
        let replies = bodies
            .into_iter()
            .map(|body| {
                Ok(PromptApiResponse {
                    status: 200,
                    body: body.as_bytes().to_vec(),
                })
            })
            .collect();
        Self {
            replies: Arc::new(Mutex::new(replies)),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, request: &PromptApiRequest) -> Result<PromptApiResponse, io::Error> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(io::Error::other("no scripted response left")))
    }

    fn calls(&self) -> Vec<PromptApiRequest> {
        self.calls.lock().expect("call lock").clone()
    }
}

impl PromptApiTransport for FakePromptApi {
    type Error = io::Error;

    fn get(&self, request: &PromptApiRequest) -> Result<PromptApiResponse, io::Error> {
        self.record(request)
    }

    fn get_async(
        &self,
        request: &PromptApiRequest,
    ) -> impl Future<Output = Result<PromptApiResponse, io::Error>> + Send {
        let reply = self.record(request);
        async move { reply }
    }
}

fn generic_manager(transport: FakePromptApi) -> GenericPromptManager<FakePromptApi> {
    generic_manager_with_params(transport, IndexMap::new())
}

fn generic_manager_with_params(
    transport: FakePromptApi,
    provider_specific_query_params: IndexMap<String, Json>,
) -> GenericPromptManager<FakePromptApi> {
    GenericPromptManager::new(
        GenericPromptSettings {
            api_base: "http://prompts.test".to_owned(),
            api_key: None,
            timeout: Duration::from_secs(30),
            prompt_id: None,
            provider_specific_query_params,
            ignore_flags: PromptIgnoreFlags::default(),
        },
        transport,
    )
}

fn spec_with_query_params() -> PromptSpec {
    spec_with_query_region("eu")
}

fn spec_with_query_region(region: &str) -> PromptSpec {
    serde_json::from_str(&format!(
        r#"{{"prompt_id":"greeting","litellm_params":{{"prompt_integration":"generic_prompt","provider_specific_query_params":{{"region":"{region}"}}}}}}"#
    ))
    .expect("prompt spec")
}

fn settings_params(region: &str) -> IndexMap<String, Json> {
    serde_json::from_str(&format!(r#"{{"region":"{region}"}}"#)).expect("settings params")
}

#[tokio::test]
async fn generic_hooks_send_the_fetched_template_with_the_client_messages() {
    let transport = FakePromptApi::replying(BODY);
    let manager = generic_manager(transport.clone());
    let dynamic = json("{}");

    let outcome = manager
        .get_chat_completion_prompt(request(Some("greeting"), "generic_prompt/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(
        outcome.messages,
        vec![
            message("system", "Be brief"),
            message("user", "client question"),
        ]
    );
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.5"))
    );
    assert_eq!(transport.calls().len(), 1);

    let asynchronous_transport = FakePromptApi::replying(BODY);
    let asynchronous_manager = generic_manager(asynchronous_transport.clone());
    let asynchronous_outcome = asynchronous_manager
        .async_completion_prompt(request(Some("greeting"), "generic_prompt/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(asynchronous_outcome.model, "gpt-4o");
    assert_eq!(
        asynchronous_outcome.messages,
        vec![
            message("system", "Be brief"),
            message("user", "client question"),
        ]
    );
    assert_eq!(asynchronous_transport.calls().len(), 1);
}

#[tokio::test]
async fn generic_strips_its_prefix_when_the_template_has_no_model() {
    let transport = FakePromptApi::replying(BODY_WITHOUT_MODEL);
    let manager = generic_manager(transport);
    let dynamic = json("{}");

    let outcome = manager
        .async_completion_prompt(request(Some("greeting"), "generic_prompt/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4");
    assert_eq!(
        outcome.messages,
        vec![
            message("user", "Say hello"),
            message("user", "client question")
        ]
    );
}

#[tokio::test]
async fn generic_async_hook_reads_provider_params_from_the_prompt_spec() {
    let transport = FakePromptApi::replying(BODY);
    let manager = generic_manager(transport.clone());
    let dynamic = json("{}");
    let spec = spec_with_query_params();
    let mut completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    manager
        .async_completion_prompt(completion)
        .await
        .expect("completion prompt");

    let calls = transport.calls();
    assert!(
        calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "region" && value == "eu")
    );
}

#[tokio::test]
async fn generic_sync_hook_compiles_without_the_prompt_spec() {
    let transport = FakePromptApi::replying(BODY);
    let manager = generic_manager(transport.clone());
    let dynamic = json("{}");
    let spec = spec_with_query_params();
    let mut completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    let calls = transport.calls();
    assert!(!calls[0].query.iter().any(|(key, _value)| key == "region"));
    assert!(
        calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "prompt_id" && value == "greeting")
    );
}

#[tokio::test]
async fn generic_sync_hook_forwards_the_callers_ignore_flags() {
    let transport = FakePromptApi::replying(BODY);
    let manager = generic_manager(transport);
    let dynamic = json("{}");
    let mut completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "generic_prompt/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
}

#[tokio::test]
async fn generic_sync_hook_fetches_with_the_managers_own_query_params() {
    let transport = FakePromptApi::replying(BODY);
    let manager = generic_manager_with_params(transport.clone(), settings_params("eu"));
    let dynamic = json("{}");
    let spec = spec_with_query_region("us");
    let mut completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "region" && value == "eu")
    );
    assert!(
        !calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "region" && value == "us")
    );
}

#[tokio::test]
async fn generic_async_hook_fetches_without_a_prompt_id_when_the_spec_has_query_params() {
    let transport = FakePromptApi::replying(BODY);
    let manager = generic_manager(transport.clone());
    let dynamic = json("{}");
    let spec = spec_with_query_params();
    let mut completion = request(None, "generic_prompt/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    let outcome = manager
        .async_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4o");
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "region" && value == "eu")
    );
    assert!(
        calls[0]
            .query
            .iter()
            .any(|(key, value)| key == "prompt_id" && value.is_empty())
    );
}

#[tokio::test]
async fn generic_sync_hook_applies_the_prompt_variables() {
    let body = r#"{"prompt_template":[{"role":"user","content":"Hello {name}"}]}"#;
    let transport = FakePromptApi::replying(body);
    let manager = generic_manager(transport);
    let dynamic = json("{}");
    let variables = params(r#"{"name":"Ann"}"#);
    let mut completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    completion.prompt_variables = Some(&variables);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(
        outcome.messages,
        vec![
            message("user", "Hello Ann"),
            message("user", "client question"),
        ]
    );
}

#[tokio::test]
async fn generic_sync_hook_keeps_versions_in_separate_cache_entries() {
    let first = r#"{"prompt_template":[{"role":"user","content":"first version"}]}"#;
    let second = r#"{"prompt_template":[{"role":"user","content":"second version"}]}"#;
    let transport = FakePromptApi::replying_many(vec![first, second]);
    let manager = generic_manager(transport.clone());
    let dynamic = json("{}");

    let mut first_completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    first_completion.prompt_version = Some(1);
    let outcome = manager
        .get_chat_completion_prompt(first_completion)
        .await
        .expect("first completion prompt");
    assert_eq!(outcome.messages[0], message("user", "first version"));

    let mut second_completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    second_completion.prompt_version = Some(2);
    let second_outcome = manager
        .get_chat_completion_prompt(second_completion)
        .await
        .expect("second completion prompt");
    assert_eq!(
        second_outcome.messages[0],
        message("user", "second version")
    );
    assert_eq!(transport.calls().len(), 2);
}

#[tokio::test]
async fn generic_sync_hook_keeps_labels_in_separate_cache_entries() {
    let first = r#"{"prompt_template":[{"role":"user","content":"production prompt"}]}"#;
    let second = r#"{"prompt_template":[{"role":"user","content":"staging prompt"}]}"#;
    let transport = FakePromptApi::replying_many(vec![first, second]);
    let manager = generic_manager(transport.clone());
    let dynamic = json("{}");

    let mut first_completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    first_completion.prompt_label = Some("production");
    let outcome = manager
        .get_chat_completion_prompt(first_completion)
        .await
        .expect("first completion prompt");
    assert_eq!(outcome.messages[0], message("user", "production prompt"));

    let mut second_completion = request(Some("greeting"), "generic_prompt/gpt-4", &dynamic);
    second_completion.prompt_label = Some("staging");
    let second_outcome = manager
        .get_chat_completion_prompt(second_completion)
        .await
        .expect("second completion prompt");
    assert_eq!(
        second_outcome.messages[0],
        message("user", "staging prompt")
    );
    assert_eq!(transport.calls().len(), 2);
}
