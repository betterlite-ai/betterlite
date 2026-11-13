#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::custom::RoleTemplate;
use betterlite_prompts::factory::hub::{ChatTemplate, Hub, TokenizerConfig, TokenizerDocument};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::{
    CustomPromptEntry, RequestError, RequestInput, transform_request, validate_environment,
};
use crate::openai_base_config::Headers;
use crate::sagemaker_completion::params::Options;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("the fixed test time")
}

fn template(text: &str) -> TokenizerConfig {
    TokenizerConfig::Success(Some(TokenizerDocument {
        chat_template: ChatTemplate::Text(text.to_owned()),
        bos_token: None,
        eos_token: None,
    }))
}

fn messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("the test messages")
}

fn entry(initial: &str, final_value: &str) -> CustomPromptEntry {
    CustomPromptEntry {
        roles: BTreeMap::from([(
            String::from("user"),
            RoleTemplate {
                pre_message: String::from("U: "),
                post_message: String::new(),
            },
        )]),
        initial_prompt_value: Some(initial.to_owned()),
        final_prompt_value: Some(final_value.to_owned()),
    }
}

struct Fixture {
    prompts: PromptFactory,
    custom: BTreeMap<String, CustomPromptEntry>,
    global: BTreeMap<String, CustomPromptEntry>,
    options: Options,
    messages: Vec<Message>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            prompts: PromptFactory {
                cache: Mutex::new(BTreeMap::from([
                    (String::from("meta-llama/Llama-2-7b"), template("BASE")),
                    (String::from("plain-model"), template("PLAIN")),
                    (String::from("hf-endpoint"), template("HF")),
                ])),
                hub: Hub::Unreachable,
            },
            custom: BTreeMap::new(),
            global: BTreeMap::new(),
            options: Options::new(),
            messages: messages(),
        }
    }

    fn request<'fixture>(
        &'fixture self,
        model: &'fixture str,
        hf_model_name: Option<&'fixture str>,
    ) -> RequestInput<'fixture> {
        RequestInput {
            model,
            messages: &self.messages,
            optional_params: &self.options,
            custom_prompt_dict: &self.custom,
            global_custom_prompt_dict: &self.global,
            hf_model_name,
            prompts: &self.prompts,
            now: now(),
        }
    }
}

async fn body(input: &RequestInput<'_>) -> IndexMap<String, Value> {
    transform_request(input).await.expect("the body")
}

fn keys(body: &IndexMap<String, Value>) -> Vec<&str> {
    body.keys().map(String::as_str).collect()
}

#[tokio::test]
async fn a_model_entry_in_the_request_dictionary_builds_the_prompt() {
    let mut fixture = Fixture::new();
    fixture
        .custom
        .insert(String::from("endpoint"), entry("<i>", "<f>"));
    let built = body(&fixture.request("endpoint", None)).await;
    assert_eq!(built.get("inputs"), Some(&json!("<i>U: hi<f>")));
}

#[tokio::test]
async fn the_hf_model_name_entry_builds_the_prompt_when_the_model_has_none() {
    let mut fixture = Fixture::new();
    fixture
        .custom
        .insert(String::from("hf-endpoint"), entry("[", "]"));
    let built = body(&fixture.request("endpoint", Some("hf-endpoint"))).await;
    assert_eq!(built.get("inputs"), Some(&json!("[U: hi]")));
}

#[tokio::test]
async fn the_global_dictionary_is_used_when_the_request_has_none() {
    let mut fixture = Fixture::new();
    fixture
        .global
        .insert(String::from("endpoint"), entry("(", ")"));
    let built = body(&fixture.request("endpoint", None)).await;
    assert_eq!(built.get("inputs"), Some(&json!("(U: hi)")));
}

#[tokio::test]
async fn the_request_dictionary_wins_over_the_global_one() {
    let mut fixture = Fixture::new();
    fixture
        .custom
        .insert(String::from("endpoint"), entry("[", "]"));
    fixture
        .global
        .insert(String::from("endpoint"), entry("(", ")"));
    let built = body(&fixture.request("endpoint", None)).await;
    assert_eq!(built.get("inputs"), Some(&json!("[U: hi]")));
}

#[tokio::test]
async fn a_llama_2_chat_model_name_picks_the_chat_template() {
    let fixture = Fixture::new();
    let built = body(&fixture.request("acme/llama-2-13b-chat", None)).await;
    let prompt = built["inputs"].as_str().expect("a prompt");
    assert!(prompt.starts_with("<s>"));
    assert!(prompt.contains("[INST]"));
}

#[tokio::test]
async fn a_llama_2_model_name_without_chat_picks_the_base_template() {
    let fixture = Fixture::new();
    let built = body(&fixture.request("acme/llama-2-13b", None)).await;
    assert_eq!(built.get("inputs"), Some(&json!("BASE")));
}

#[tokio::test]
async fn an_hf_model_name_overrides_the_llama_2_fallback() {
    let fixture = Fixture::new();
    let built = body(&fixture.request("acme/llama-2-13b", Some("hf-endpoint"))).await;
    assert_eq!(built.get("inputs"), Some(&json!("HF")));
}

#[tokio::test]
async fn a_plain_model_name_goes_to_the_prompt_factory() {
    let fixture = Fixture::new();
    let built = body(&fixture.request("plain-model", None)).await;
    assert_eq!(built.get("inputs"), Some(&json!("PLAIN")));
}

#[tokio::test]
async fn the_body_keeps_the_pinned_key_order() {
    let mut fixture = Fixture::new();
    fixture.options.insert(String::from("stream"), json!(true));
    let built = body(&fixture.request("plain-model", None)).await;
    assert_eq!(keys(&built), ["parameters", "stream", "inputs"]);
}

#[tokio::test]
async fn a_stream_flag_other_than_true_leaves_the_body_without_stream() {
    let mut fixture = Fixture::new();
    fixture.options.insert(String::from("stream"), json!(false));
    let built = body(&fixture.request("plain-model", None)).await;
    assert_eq!(keys(&built), ["parameters", "inputs"]);
    assert_eq!(built["parameters"], json!({}));
}

#[tokio::test]
async fn an_integer_one_stream_flag_is_not_the_body_flag() {
    let mut fixture = Fixture::new();
    fixture.options.insert(String::from("stream"), json!(1));
    let built = body(&fixture.request("plain-model", None)).await;
    assert_eq!(keys(&built), ["parameters", "inputs"]);
}

#[tokio::test]
async fn the_other_options_land_under_parameters() {
    let mut fixture = Fixture::new();
    fixture
        .options
        .insert(String::from("temperature"), json!(0.7));
    fixture.options.insert(String::from("stop"), json!(["END"]));
    let built = body(&fixture.request("plain-model", None)).await;
    assert_eq!(
        built.get("parameters"),
        Some(&json!({"temperature": 0.7, "stop": ["END"]}))
    );
}

#[tokio::test]
async fn an_unknown_model_falls_back_to_the_default_prompt() {
    let fixture = Fixture::new();
    let built = body(&fixture.request("unknown-endpoint", None)).await;
    assert!(built.contains_key("inputs"));
}

#[tokio::test]
async fn a_model_shape_without_a_prompt_is_an_error() {
    let fixture = Fixture::new();
    let error = transform_request(&fixture.request("tiiuae/falcon-40b", None))
        .await
        .expect_err("no prompt");
    assert!(matches!(error, RequestError::NoTemplate { .. }));
    assert_eq!(
        error.to_string(),
        "the tiiuae/falcon-40b model has no chat prompt template"
    );
}

#[tokio::test]
async fn validate_environment_keeps_only_the_content_type() {
    let headers = validate_environment();
    assert_eq!(
        headers,
        Headers::from([(
            String::from("Content-Type"),
            String::from("application/json")
        )])
    );
}
