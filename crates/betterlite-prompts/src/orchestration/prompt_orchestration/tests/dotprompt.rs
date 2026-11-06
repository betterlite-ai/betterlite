use betterlite_models::prompt_contracts::PromptIgnoreFlags;

use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_compilation::init::manager::{DotpromptManager, PromptDataInput};
use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{json, message, params, request};

fn manager(content: &str, template_model: Option<&str>) -> DotpromptManager {
    let mut metadata = Object::new();
    if let Some(model) = template_model {
        metadata.insert("model".to_owned(), Json::Text(model.to_owned()));
    }
    let mut record = Object::new();
    record.insert("content".to_owned(), Json::Text(content.to_owned()));
    record.insert("metadata".to_owned(), Json::Object(metadata));
    let mut data = Object::new();
    data.insert("greeting".to_owned(), Json::Object(record));
    DotpromptManager::new(
        None,
        None,
        Some(PromptDataInput::Object(data)),
        None,
        &GlobalPromptDirectory::default(),
    )
    .expect("prompt data")
}

#[tokio::test]
async fn dotprompt_hooks_append_the_client_messages_to_the_rendered_template() {
    let manager = manager("System: be brief\nUser: hello {{ name }}", Some("gpt-4"));
    let dynamic = json("{}");
    let variables = params(r#"{"name":"Ann"}"#);
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.prompt_variables = Some(&variables);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4");
    assert_eq!(
        outcome.messages,
        vec![
            message("system", "be brief"),
            message("user", "hello Ann"),
            message("user", "client question"),
        ]
    );

    let mut asynchronous = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    asynchronous.prompt_variables = Some(&variables);
    let asynchronous_outcome = manager
        .async_completion_prompt(asynchronous)
        .await
        .expect("completion prompt");

    assert_eq!(asynchronous_outcome.model, "gpt-4");
    assert_eq!(
        asynchronous_outcome.messages,
        vec![
            message("system", "be brief"),
            message("user", "hello Ann"),
            message("user", "client question"),
        ]
    );
}

#[tokio::test]
async fn dotprompt_sync_hook_forwards_the_callers_ignore_flags() {
    let manager = manager("User: hi", Some("gpt-4"));
    let dynamic = json("{}");
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "dotprompt/gpt-4");
}

#[tokio::test]
async fn dotprompt_strips_its_prefix_when_the_template_has_no_model() {
    let manager = manager("User: hi", None);
    let dynamic = json("{}");

    let outcome = manager
        .async_completion_prompt(request(Some("greeting"), "dotprompt/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4");
    assert_eq!(
        outcome.messages,
        vec![message("user", "hi"), message("user", "client question")]
    );
}
