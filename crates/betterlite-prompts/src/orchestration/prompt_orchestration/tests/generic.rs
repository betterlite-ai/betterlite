use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::generic_prompt_api::manager::{GenericPromptManager, GenericPromptSettings};
use betterlite_models::generic_prompt_api::request::{
    PromptApiRequest, PromptApiResponse, PromptApiTransport,
};
use betterlite_models::prompt_contracts::PromptIgnoreFlags;
use betterlite_models::prompt_spec::PromptSpec;
use indexmap::IndexMap;

use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{json, message, request};

const BODY: &str = r#"{"prompt_template":[{"role":"system","content":"Be brief"}],"prompt_template_model":"gpt-4o","prompt_template_optional_params":{"temperature":0.5}}"#;
const BODY_WITHOUT_MODEL: &str = r#"{"prompt_template":[{"role":"user","content":"Say hello"}]}"#;

#[derive(Clone)]
struct FakePromptApi {
    replies: Arc<Mutex<VecDeque<Result<PromptApiResponse, io::Error>>>>,
    calls: Arc<Mutex<Vec<PromptApiRequest>>>,
}

impl FakePromptApi {
    fn replying(body: &str) -> Self {
        let reply = PromptApiResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        };
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(vec![Ok(reply)]))),
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
    GenericPromptManager::new(
        GenericPromptSettings {
            api_base: "http://prompts.test".to_owned(),
            api_key: None,
            timeout: Duration::from_secs(30),
            prompt_id: None,
            provider_specific_query_params: IndexMap::new(),
            ignore_flags: PromptIgnoreFlags::default(),
        },
        transport,
    )
}

fn spec_with_query_params() -> PromptSpec {
    serde_json::from_str(
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"generic_prompt","provider_specific_query_params":{"region":"eu"}}}"#,
    )
    .expect("prompt spec")
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
