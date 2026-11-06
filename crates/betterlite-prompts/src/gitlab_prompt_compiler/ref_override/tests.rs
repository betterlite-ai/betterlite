#![cfg(test)]

use std::io;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptCompileRequest;
use indexmap::IndexMap;

use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;
use crate::gitlab_prompt_compiler::init::manager::GitLabPromptManager;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

type Scripted = Vec<Result<GitLabResponse, io::Error>>;

fn manager(
    ref_name: Option<&str>,
    responses: Scripted,
) -> (GitLabPromptManager<FakeTransport>, FakeTransport) {
    let transport = FakeTransport::scripted(responses);
    let manager = GitLabPromptManager::new(
        config_from(CONFIG),
        None,
        ref_name.map(str::to_owned),
        None,
        transport.clone(),
    )
    .expect("the config builds a client");
    (manager, transport)
}

fn dynamic_params(git_ref: ContractJson) -> ContractJson {
    ContractJson::Object(IndexMap::from([(
        "extra".to_owned(),
        ContractJson::Object(IndexMap::from([("git_ref".to_owned(), git_ref)])),
    )]))
}

fn request<'request>(
    prompt_id: &'request str,
    dynamic_callback_params: &'request ContractJson,
) -> PromptCompileRequest<'request> {
    PromptCompileRequest {
        prompt_id: Some(prompt_id),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params,
        prompt_label: None,
        prompt_version: None,
    }
}

#[tokio::test]
async fn a_dynamic_git_ref_overrides_the_stored_ref_for_the_load() {
    let (manager, transport) = manager(Some("stored"), vec![answer!(200, "User: hi")]);
    let dynamic = dynamic_params(ContractJson::Text("feature".to_owned()));

    manager
        .compile_prompt_helper(&request("chat/hi", &dynamic))
        .await
        .unwrap();

    assert!(transport.requests()[0].url.ends_with("?ref=feature"));
}

#[tokio::test]
async fn without_a_dynamic_git_ref_the_stored_ref_is_used() {
    let (manager, transport) = manager(Some("stored"), vec![answer!(200, "User: hi")]);
    let dynamic = ContractJson::Null;

    manager
        .compile_prompt_helper(&request("chat/hi", &dynamic))
        .await
        .unwrap();

    assert!(transport.requests()[0].url.ends_with("?ref=stored"));
}

#[tokio::test]
async fn an_empty_dynamic_git_ref_falls_back_to_the_stored_ref() {
    let (manager, transport) = manager(Some("stored"), vec![answer!(200, "User: hi")]);
    let dynamic = dynamic_params(ContractJson::Text(String::new()));

    manager
        .compile_prompt_helper(&request("chat/hi", &dynamic))
        .await
        .unwrap();

    assert!(transport.requests()[0].url.ends_with("?ref=stored"));
}

#[tokio::test]
async fn an_encoded_id_uses_the_dynamic_ref_first_and_the_stored_ref_second() {
    let (manager, transport) = manager(
        Some("stored"),
        vec![answer!(200, "User: hi"), answer!(200, "User: hi")],
    );
    let dynamic = dynamic_params(ContractJson::Text("feature".to_owned()));

    manager
        .compile_prompt_helper(&request("gitlab::chat::hi", &dynamic))
        .await
        .unwrap();

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].url.ends_with("?ref=feature"));
    assert!(requests[1].url.ends_with("?ref=stored"));
}

#[tokio::test]
async fn an_unknown_dynamic_key_changes_nothing() {
    let (manager, transport) = manager(Some("stored"), vec![answer!(200, "User: hi")]);
    let dynamic = ContractJson::Object(IndexMap::from([(
        "extra".to_owned(),
        ContractJson::Text("not a mapping".to_owned()),
    )]));

    manager
        .compile_prompt_helper(&request("chat/hi", &dynamic))
        .await
        .unwrap();

    assert!(transport.requests()[0].url.ends_with("?ref=stored"));
}
