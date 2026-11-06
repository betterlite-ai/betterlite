#![cfg(test)]

use std::io;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{
    PromptCompileRequest, PromptManagementClient, PromptParams,
};

use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;
use crate::gitlab_prompt_compiler::error::CompileError;
use crate::gitlab_prompt_compiler::init::manager::GitLabPromptManager;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

type Scripted = Vec<Result<GitLabResponse, io::Error>>;

async fn manager(responses: Scripted) -> (GitLabPromptManager<FakeTransport>, FakeTransport) {
    let transport = FakeTransport::scripted(responses);
    let manager =
        GitLabPromptManager::new(config_from(CONFIG), None, None, None, transport.clone())
            .await
            .expect("the config builds a client");
    (manager, transport)
}

fn request<'request>(
    prompt_id: Option<&'request str>,
    prompt_variables: Option<&'request PromptParams>,
    dynamic_callback_params: &'request ContractJson,
) -> PromptCompileRequest<'request> {
    PromptCompileRequest {
        prompt_id,
        prompt_spec: None,
        prompt_variables,
        dynamic_callback_params,
        prompt_label: None,
        prompt_version: None,
    }
}

#[tokio::test]
async fn a_missing_prompt_id_is_rejected() {
    let (manager, transport) = manager(vec![]).await;
    let dynamic = ContractJson::Null;

    let error = manager
        .compile_prompt_helper(&request(None, None, &dynamic))
        .await
        .unwrap_err();

    assert!(matches!(*error, CompileError::PromptIdRequired));
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn a_compiled_prompt_carries_messages_model_and_optional_params() {
    let template = "---\nmodel: gpt-4o\ntemperature: 0.5\nmax_tokens: 100\ntop_p: 0.9\n---\nSystem: be brief\nUser: hello {{ name }}";
    let (manager, _transport) = manager(vec![answer!(200, template)]).await;
    let variables: PromptParams = serde_json::from_str(r#"{"name": "Ada"}"#).unwrap();
    let dynamic = ContractJson::Null;

    let client: PromptManagementClient = manager
        .compile_prompt_helper(&request(Some("chat/hi"), Some(&variables), &dynamic))
        .await
        .unwrap();

    let expected: Vec<ContractJson> = serde_json::from_str(
        r#"[{"role":"system","content":"be brief"},{"role":"user","content":"hello Ada"}]"#,
    )
    .unwrap();
    assert_eq!(client.prompt_template, expected);
    assert_eq!(client.prompt_id.as_deref(), Some("chat/hi"));
    assert_eq!(client.prompt_template_model.as_deref(), Some("gpt-4o"));
    assert_eq!(client.completed_messages, None);

    let params = client.prompt_optional_params.unwrap();
    let keys: Vec<&str> = params.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["temperature", "max_tokens", "top_p"]);
    assert_eq!(
        serde_json::to_string(params.get("temperature").unwrap()).unwrap(),
        "0.5"
    );
    assert_eq!(
        serde_json::to_string(params.get("max_tokens").unwrap()).unwrap(),
        "100"
    );
    assert_eq!(
        serde_json::to_string(params.get("top_p").unwrap()).unwrap(),
        "0.9"
    );
}

#[tokio::test]
async fn a_compile_failure_names_the_requested_prompt() {
    let (manager, _transport) = manager(vec![
        answer!(404, ""),
        answer!(404, ""),
        answer!(404, ""),
        answer!(404, ""),
    ])
    .await;
    let dynamic = ContractJson::Null;

    let error = manager
        .compile_prompt_helper(&request(Some("missing"), None, &dynamic))
        .await
        .unwrap_err();

    assert!(matches!(&*error, CompileError::Compile { .. }));
    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'missing': Prompt template 'missing' not found"
    );
}

#[tokio::test]
async fn an_encoded_id_is_loaded_under_the_decoded_id_first() {
    let template = "User: hi";
    let (manager, transport) = manager(vec![answer!(200, template), answer!(200, template)]).await;
    let dynamic = ContractJson::Null;

    let client = manager
        .compile_prompt_helper(&request(Some("gitlab::chat::hi"), None, &dynamic))
        .await
        .unwrap();

    assert_eq!(client.prompt_id.as_deref(), Some("gitlab::chat::hi"));
    assert_eq!(transport.requests().len(), 2);
    assert!(
        transport
            .requests()
            .iter()
            .all(|request| request.url.contains("chat%2Fhi.prompt"))
    );
}

#[tokio::test]
async fn a_client_without_optional_params_still_reports_temperature_and_max_tokens() {
    let (manager, _transport) = manager(vec![answer!(200, "User: hi")]).await;
    let dynamic = ContractJson::Null;

    let client = manager
        .compile_prompt_helper(&request(Some("chat/hi"), None, &dynamic))
        .await
        .unwrap();

    let params = client.prompt_optional_params.unwrap();
    let keys: Vec<&str> = params.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["temperature", "max_tokens"]);
    assert!(params.values().all(|value| *value == ContractJson::Null));
}
