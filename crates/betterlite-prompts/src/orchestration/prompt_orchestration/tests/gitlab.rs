use std::io;

use betterlite_models::prompt_contracts::PromptIgnoreFlags;

use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;
use crate::gitlab_prompt_compiler::init::manager::GitLabPromptManager;
use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{ignoring_all, json, message, prompt_spec, request};

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;
const TEMPLATE: &str = "---\nmodel: gpt-4o\ntemperature: 0.5\n---\nSystem: be brief\nUser: hello";

async fn gitlab_manager(
    responses: Vec<Result<GitLabResponse, io::Error>>,
) -> GitLabPromptManager<FakeTransport> {
    let transport = FakeTransport::scripted(responses);
    GitLabPromptManager::new(config_from(CONFIG), None, None, None, transport)
        .await
        .expect("the config builds a client")
}

#[tokio::test]
async fn gitlab_hooks_append_the_client_messages_to_the_parsed_template() {
    let manager = gitlab_manager(vec![answer!(200, TEMPLATE)]).await;
    let dynamic = json("{}");

    let outcome = manager
        .get_chat_completion_prompt(request(Some("chat/hi"), "gitlab/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(
        outcome.messages,
        vec![
            message("system", "be brief"),
            message("user", "hello"),
            message("user", "client question"),
        ]
    );
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.5"))
    );
}

#[tokio::test]
async fn gitlab_sync_hook_drops_the_callers_ignore_flags() {
    let manager = gitlab_manager(vec![answer!(200, TEMPLATE)]).await;
    let dynamic = json("{}");
    let mut completion = request(Some("chat/hi"), "gitlab/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.5"))
    );
}

#[tokio::test]
async fn gitlab_sync_hook_still_resolves_the_ignore_flags_from_the_prompt_spec() {
    let manager = gitlab_manager(vec![answer!(200, TEMPLATE)]).await;
    let dynamic = json("{}");
    let spec = prompt_spec(ignoring_all());
    let mut completion = request(Some("chat/hi"), "gitlab/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gitlab/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
}

#[tokio::test]
async fn gitlab_async_hook_honors_the_callers_ignore_flags() {
    let manager = gitlab_manager(vec![answer!(200, TEMPLATE)]).await;
    let dynamic = json("{}");
    let mut completion = request(Some("chat/hi"), "gitlab/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = manager
        .async_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gitlab/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
}
