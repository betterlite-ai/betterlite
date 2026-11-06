#![cfg(test)]

use std::io;

use betterlite_models::prompt_contracts::PromptParams;

use crate::arguments::json::Json;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;
use crate::gitlab_prompt_compiler::error::GetPromptError;
use crate::gitlab_prompt_compiler::init::manager::GitLabPromptManager;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

type Scripted = Vec<Result<GitLabResponse, io::Error>>;

fn manager(
    prompt_id: Option<&str>,
    ref_name: Option<&str>,
    responses: Scripted,
) -> (GitLabPromptManager<FakeTransport>, FakeTransport) {
    let transport = FakeTransport::scripted(responses);
    let manager = GitLabPromptManager::new(
        config_from(CONFIG),
        prompt_id.map(str::to_owned),
        ref_name.map(str::to_owned),
        None,
        transport.clone(),
    )
    .expect("the config builds a client");
    (manager, transport)
}

#[tokio::test]
async fn the_template_is_rendered_with_the_prompt_variables() {
    let template = "---\nmodel: gpt-4o\ntemperature: 0.5\n---\nUser: hello {{ name }}";
    let (manager, _transport) = manager(None, None, vec![answer!(200, template)]);
    let variables: PromptParams = serde_json::from_str(r#"{"name": "Ada"}"#).unwrap();

    let (rendered, metadata) = manager
        .get_prompt_template("chat/hi", Some(&variables), None)
        .await
        .unwrap();

    assert_eq!(rendered, "User: hello Ada");
    assert_eq!(metadata.get("model").and_then(Json::as_str), Some("gpt-4o"));
    assert_eq!(
        serde_json::to_string(metadata.get("temperature").unwrap()).unwrap(),
        "0.5"
    );
    assert_eq!(metadata.get("max_tokens"), Some(&Json::Null));
}

#[tokio::test]
async fn a_loaded_prompt_is_not_fetched_again() {
    let (manager, transport) = manager(None, None, vec![answer!(200, "User: hi")]);

    manager
        .get_prompt_template("chat/hi", None, None)
        .await
        .unwrap();
    manager
        .get_prompt_template("chat/hi", None, None)
        .await
        .unwrap();

    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn a_missing_template_is_reported_by_name() {
    let (manager, _transport) = manager(None, None, vec![answer!(404, ""), answer!(404, "")]);

    let error = manager
        .get_prompt_template("missing", None, None)
        .await
        .unwrap_err();

    assert!(matches!(
        &error,
        GetPromptError::TemplateNotFound { prompt_id } if prompt_id.as_str() == "missing"
    ));
    assert_eq!(error.to_string(), "Prompt template 'missing' not found");
}

#[tokio::test]
async fn the_ref_argument_overrides_the_stored_ref() {
    let (manager, transport) = manager(None, Some("stored"), vec![answer!(200, "User: hi")]);

    manager
        .get_prompt_template("chat/hi", None, Some("release/1"))
        .await
        .unwrap();

    let url = &transport.requests()[0].url;
    assert!(url.ends_with("?ref=release%2F1"), "{url}");
}

#[tokio::test]
async fn the_stored_ref_is_used_without_a_ref_argument() {
    let (manager, transport) = manager(None, Some("v2"), vec![answer!(200, "User: hi")]);

    manager
        .get_prompt_template("chat/hi", None, None)
        .await
        .unwrap();

    let url = &transport.requests()[0].url;
    assert!(url.ends_with("?ref=v2"), "{url}");
}

#[tokio::test]
async fn rendering_failures_are_reported() {
    let (manager, _transport) = manager(
        None,
        None,
        vec![answer!(200, "User: {{ missing | no_such_filter }}")],
    );

    let error = manager
        .get_prompt_template("chat/hi", None, None)
        .await
        .unwrap_err();

    assert!(matches!(error, GetPromptError::Manager(_)));
}
