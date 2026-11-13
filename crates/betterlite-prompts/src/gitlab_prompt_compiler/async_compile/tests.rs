#![cfg(test)]

use std::io;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient, PromptParams,
};

use super::async_compile_prompt_helper;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;
use crate::gitlab_prompt_compiler::error::CompileError;
use crate::gitlab_prompt_compiler::init::manager::GitLabPromptManager;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

type Scripted = Vec<Result<GitLabResponse, io::Error>>;

async fn manager(responses: Scripted) -> GitLabPromptManager<FakeTransport> {
    let transport = FakeTransport::scripted(responses);
    GitLabPromptManager::new(config_from(CONFIG), None, None, None, transport)
        .await
        .expect("the config builds a client")
}

fn async_request<'request>(
    prompt_id: Option<&'request str>,
    prompt_variables: Option<&'request PromptParams>,
    dynamic_callback_params: &'request ContractJson,
) -> PromptAsyncCompileRequest<'request> {
    PromptAsyncCompileRequest {
        prompt_id,
        prompt_variables,
        dynamic_callback_params,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
    }
}

#[tokio::test]
async fn a_missing_prompt_id_is_rejected() {
    let manager = manager(vec![]).await;
    let dynamic = ContractJson::Null;

    let error = async_compile_prompt_helper(&manager, &async_request(None, None, &dynamic))
        .await
        .unwrap_err();

    assert!(matches!(*error, CompileError::PromptIdRequired));
}

#[tokio::test]
async fn the_async_helper_returns_what_the_sync_helper_compiles() {
    let template = "---\nmodel: gpt-4o\ntemperature: 0.5\n---\nUser: hello {{ name }}";
    let manager = manager(vec![answer!(200, template)]).await;
    let variables: PromptParams = serde_json::from_str(r#"{"name": "Ada"}"#).unwrap();
    let dynamic =
        ContractJson::Object(serde_json::from_str(r#"{"extra": {"git_ref": "v2"}}"#).unwrap());

    let sync_request = PromptCompileRequest {
        prompt_id: Some("chat/hi"),
        prompt_spec: None,
        prompt_variables: Some(&variables),
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    let sync_client: PromptManagementClient =
        manager.compile_prompt_helper(&sync_request).await.unwrap();
    let async_client = async_compile_prompt_helper(
        &manager,
        &async_request(Some("chat/hi"), Some(&variables), &dynamic),
    )
    .await
    .unwrap();

    assert_eq!(async_client, sync_client);
}
