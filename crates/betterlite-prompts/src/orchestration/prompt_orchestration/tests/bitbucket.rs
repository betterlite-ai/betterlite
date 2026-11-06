use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::prompt_contracts::PromptIgnoreFlags;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};

use crate::bitbucket::manager::BitBucketPromptManager;
use crate::bitbucket_client::BitBucketClient;
use crate::bitbucket_client::config::BitBucketConfig;
use crate::bitbucket_prompt_manager::BitBucketTemplateManager;
use crate::bitbucket_prompt_manager::parse::{YamlParser, parse_prompt_file};
use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{ignoring_all, json, message, params, prompt_spec, request};

const TEMPLATE: &str = "---\nmodel: gpt-4\ntemperature: 0.5\n---\nSystem: be brief\nUser: hello";
const TEMPLATE_WITH_VARIABLES: &str =
    "---\nmodel: gpt-4\n---\nSystem: be brief\nUser: hello {{ name }}";
const TEMPLATE_WITHOUT_MODEL: &str = "---\ntemperature: 0.5\n---\nSystem: be brief\nUser: hello";

fn config() -> BitBucketConfig {
    BitBucketConfig {
        workspace: Some("ws".to_owned()),
        repository: Some("repo".to_owned()),
        access_token: Some(SecretString::from("token")),
        branch: None,
        base_url: Some("http://127.0.0.1:1/2.0".to_owned()),
        auth_method: None,
        username: None,
        unrecognized: BTreeMap::new(),
    }
}

fn transport() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy policy"),
    );
    ProxiedClient::outbound(&policy).expect("transport")
}

// The template lands in the manager's startup cache, so the hooks never reach BitBucket.
fn bitbucket_manager(template: &str) -> BitBucketPromptManager {
    let mut prompts = IndexMap::new();
    prompts.insert(
        "greeting".to_owned(),
        parse_prompt_file(template, "greeting", YamlParser::default()).expect("template"),
    );
    let template_manager = BitBucketTemplateManager {
        prompts,
        yaml_parser: YamlParser::default(),
        client: BitBucketClient::new(config(), transport()).expect("bitbucket client"),
    };
    BitBucketPromptManager {
        bitbucket_config: config(),
        prompt_id: None,
        transport: transport(),
        template_manager: OnceCell::from(AsyncMutex::new(template_manager)),
    }
}

#[tokio::test]
async fn bitbucket_hooks_append_the_client_messages_to_the_parsed_template() {
    let manager = bitbucket_manager(TEMPLATE);
    let dynamic = json("{}");

    let outcome = manager
        .get_chat_completion_prompt(request(Some("greeting"), "bitbucket/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4");
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

    let asynchronous_outcome = manager
        .async_completion_prompt(request(Some("greeting"), "bitbucket/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(asynchronous_outcome.model, "gpt-4");
    assert_eq!(
        asynchronous_outcome.messages,
        vec![
            message("system", "be brief"),
            message("user", "hello"),
            message("user", "client question"),
        ]
    );
}

#[tokio::test]
async fn bitbucket_hooks_render_the_prompt_variables() {
    let manager = bitbucket_manager(TEMPLATE_WITH_VARIABLES);
    let dynamic = json("{}");
    let variables = params(r#"{"name":"Ann"}"#);
    let mut completion = request(Some("greeting"), "bitbucket/gpt-4", &dynamic);
    completion.prompt_variables = Some(&variables);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(
        outcome.messages,
        vec![
            message("system", "be brief"),
            message("user", "hello Ann"),
            message("user", "client question"),
        ]
    );
}

#[tokio::test]
async fn bitbucket_hooks_strip_the_integration_prefix_when_the_template_has_no_model() {
    let manager = bitbucket_manager(TEMPLATE_WITHOUT_MODEL);
    let dynamic = json("{}");

    let outcome = manager
        .get_chat_completion_prompt(request(Some("greeting"), "bitbucket/gpt-4", &dynamic))
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4");
    assert_eq!(
        outcome.messages,
        vec![
            message("system", "be brief"),
            message("user", "hello"),
            message("user", "client question"),
        ]
    );
}

#[tokio::test]
async fn bitbucket_sync_hook_drops_the_callers_ignore_flags() {
    let manager = bitbucket_manager(TEMPLATE);
    let dynamic = json("{}");
    let mut completion = request(Some("greeting"), "bitbucket/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.5"))
    );
}

#[tokio::test]
async fn bitbucket_sync_hook_still_resolves_the_ignore_flags_from_the_prompt_spec() {
    let manager = bitbucket_manager(TEMPLATE);
    let dynamic = json("{}");
    let spec = prompt_spec(ignoring_all());
    let mut completion = request(Some("greeting"), "bitbucket/gpt-4", &dynamic);
    completion.prompt_spec = Some(&spec);

    let outcome = manager
        .get_chat_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "bitbucket/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
}

#[tokio::test]
async fn bitbucket_async_hook_honors_the_callers_ignore_flags() {
    let manager = bitbucket_manager(TEMPLATE);
    let dynamic = json("{}");
    let mut completion = request(Some("greeting"), "bitbucket/gpt-4", &dynamic);
    completion.ignore_flags = PromptIgnoreFlags {
        model: true,
        optional_params: true,
    };

    let outcome = manager
        .async_completion_prompt(completion)
        .await
        .expect("completion prompt");

    assert_eq!(outcome.model, "bitbucket/gpt-4");
    assert_eq!(
        outcome.non_default_params.get("temperature"),
        Some(&json("0.2"))
    );
}
