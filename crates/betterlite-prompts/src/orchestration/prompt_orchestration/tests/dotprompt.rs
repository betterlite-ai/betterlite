use betterlite_models::prompt_contracts::PromptIgnoreFlags;

use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_compilation::init::manager::{DotpromptManager, PromptDataInput};
use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{client_messages, json, message, params, request};

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

fn manager_with_two_versions() -> DotpromptManager {
    let mut base_record = Object::new();
    base_record.insert("content".to_owned(), Json::Text("User: base".to_owned()));
    base_record.insert("metadata".to_owned(), Json::Object(Object::new()));
    let mut versioned_record = Object::new();
    versioned_record.insert(
        "content".to_owned(),
        Json::Text("User: version two".to_owned()),
    );
    versioned_record.insert("metadata".to_owned(), Json::Object(Object::new()));
    let mut data = Object::new();
    data.insert("greeting".to_owned(), Json::Object(base_record));
    data.insert("greeting.v2".to_owned(), Json::Object(versioned_record));
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
async fn dotprompt_hooks_render_the_requested_template_version() {
    let manager = manager_with_two_versions();
    let dynamic = json("{}");
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.prompt_version = Some(2);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(
        outcome.messages,
        vec![
            message("user", "version two"),
            message("user", "client question"),
        ]
    );

    let mut asynchronous = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    asynchronous.prompt_version = Some(2);
    let asynchronous_outcome = manager
        .async_completion_prompt(asynchronous)
        .await
        .expect("completion prompt");

    assert_eq!(
        asynchronous_outcome.messages,
        vec![
            message("user", "version two"),
            message("user", "client question"),
        ]
    );
}

#[tokio::test]
async fn dotprompt_returns_the_request_unchanged_for_an_unknown_prompt_id() {
    let manager = manager("User: hi", Some("gpt-4"));
    let dynamic = json("{}");

    let outcome = manager
        .get_chat_completion_prompt(request(Some("missing"), "dotprompt/gpt-4", &dynamic))
        .await
        .expect("the request passes through");

    assert_eq!(outcome.model, "dotprompt/gpt-4");
    assert_eq!(outcome.messages, client_messages());
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
}

#[tokio::test]
async fn dotprompt_renders_non_text_prompt_variables() {
    let manager = manager(
        concat!(
            "User: count {{ count }} big {{ big }} bigger {{ big + 1 }} ",
            "low {{ low }} lower {{ low + 1 }} flag {{ flag }} ",
            "name {{ profile.name }} second {{ items[1] }}",
        ),
        None,
    );
    let dynamic = json("{}");
    let variables = params(
        r#"{"count":3,"big":18446744073709551616,"low":-18446744073709551616,"flag":true,"profile":{"name":"Ada"},"items":["one","two"]}"#,
    );
    let mut completion = request(Some("greeting"), "dotprompt/gpt-4", &dynamic);
    completion.prompt_variables = Some(&variables);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(
        outcome.messages,
        vec![
            message(
                "user",
                concat!(
                    "count 3 big 18446744073709551616 bigger 18446744073709551617 ",
                    "low -18446744073709551616 lower -18446744073709551615 ",
                    "flag True name Ada second two",
                ),
            ),
            message("user", "client question"),
        ]
    );
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
