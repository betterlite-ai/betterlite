#![cfg(test)]

use std::collections::BTreeMap;

use crate::databricks_auth::environment::DatabricksEnv;
use crate::databricks_auth::error::{DatabricksError, SDK_CREDENTIALS};
use crate::databricks_auth::oauth::{OauthReply, OauthTokenRequest, OauthTokenSource};
use crate::databricks_embedding::{
    DatabricksEmbeddingConfig, EmbeddingPreparation, PreparedEmbedding, prepare_embedding,
};
use async_trait::async_trait;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use tokio::runtime::{Builder, Runtime};

const VERSION: &str = "1.105.1";

struct NoTokens;

#[async_trait]
impl OauthTokenSource for NoTokens {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        OauthReply::Transport("the token source is not expected here".to_owned())
    }
}

fn env() -> DatabricksEnv {
    DatabricksEnv {
        client_id: None,
        client_secret: None,
        api_base: None,
        user_agent_override: None,
        version: VERSION.to_owned(),
    }
}

fn key() -> SecretString {
    SecretString::from("pat".to_owned())
}

fn params(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the runtime builds")
}

fn prepare(
    optional_params: Object,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
) -> Result<PreparedEmbedding, DatabricksError> {
    let runtime = runtime();
    runtime.block_on(prepare_embedding(
        EmbeddingPreparation {
            optional_params,
            api_key,
            api_base,
            custom_endpoint: false,
            headers: None,
        },
        &env(),
        &NoTokens,
    ))
}

#[test]
fn the_config_reports_only_a_set_instruction() {
    assert!(
        DatabricksEmbeddingConfig { instruction: None }
            .get_config()
            .is_empty()
    );
    assert_eq!(
        DatabricksEmbeddingConfig {
            instruction: Some("Represent this sentence".to_owned())
        }
        .get_config(),
        BTreeMap::from([(
            "instruction".to_owned(),
            "Represent this sentence".to_owned()
        )])
    );
}

#[test]
fn the_supported_params_are_empty() {
    assert!(
        DatabricksEmbeddingConfig { instruction: None }
            .get_supported_openai_params()
            .is_empty()
    );
}

#[test]
fn mapping_hands_the_params_back_unchanged() {
    let optional = params(&[("encoding_format", Json::Text("float".to_owned()))]);
    let mapped =
        DatabricksEmbeddingConfig { instruction: None }.map_openai_params(optional.clone());
    assert_eq!(mapped, optional);
}

#[test]
fn a_non_empty_user_agent_is_popped_and_databricks_user_agent_stays() {
    let pat = key();
    let prepared = prepare(
        params(&[
            ("user_agent", Json::Text("acme/1.0".to_owned())),
            ("databricks_user_agent", Json::Text("other/2.0".to_owned())),
            ("dimensions", Json::Number(Number::from(5_i32))),
        ]),
        Some(&pat),
        Some("https://host"),
    )
    .expect("the call resolves");
    assert_eq!(
        prepared.headers.get("User-Agent").map(String::as_str),
        Some("acme_litellm/1.105.1")
    );
    assert_eq!(
        prepared.optional_params.get("databricks_user_agent"),
        Some(&Json::Text("other/2.0".to_owned()))
    );
    assert!(prepared.optional_params.get("user_agent").is_none());
    assert_eq!(
        prepared.optional_params.get("dimensions"),
        Some(&Json::Number(Number::from(5_i32)))
    );
}

#[test]
fn an_empty_user_agent_pops_the_databricks_one() {
    let pat = key();
    let prepared = prepare(
        params(&[
            ("user_agent", Json::Text(String::new())),
            (
                "databricks_user_agent",
                Json::Text("dbrx_partner/2".to_owned()),
            ),
        ]),
        Some(&pat),
        Some("https://host"),
    )
    .expect("the call resolves");
    assert_eq!(
        prepared.headers.get("User-Agent").map(String::as_str),
        Some("dbrx_partner_litellm/1.105.1")
    );
    assert!(prepared.optional_params.is_empty());
}

#[test]
fn the_env_override_applies_when_no_user_agent_is_set() {
    let mut env = env();
    env.user_agent_override = Some("env_partner/3".to_owned());
    let pat = key();
    let runtime = runtime();
    let prepared = runtime
        .block_on(prepare_embedding(
            EmbeddingPreparation {
                optional_params: params(&[]),
                api_key: Some(&pat),
                api_base: Some("https://host"),
                custom_endpoint: false,
                headers: None,
            },
            &env,
            &NoTokens,
        ))
        .expect("the call resolves");
    assert_eq!(
        prepared.headers.get("User-Agent").map(String::as_str),
        Some("env_partner_litellm/1.105.1")
    );
}

#[test]
fn the_embedding_url_gets_the_embeddings_suffix() {
    let pat = key();
    let prepared =
        prepare(params(&[]), Some(&pat), Some("https://host")).expect("the call resolves");
    assert_eq!(prepared.url, "https://host/embeddings");
    assert_eq!(
        prepared.headers.get("Authorization").map(String::as_str),
        Some("Bearer pat")
    );
    assert_eq!(
        prepared.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn a_custom_embedding_endpoint_keeps_the_base() {
    let pat = key();
    let runtime = runtime();
    let prepared = runtime
        .block_on(prepare_embedding(
            EmbeddingPreparation {
                optional_params: params(&[]),
                api_key: Some(&pat),
                api_base: Some("https://host"),
                custom_endpoint: true,
                headers: None,
            },
            &env(),
            &NoTokens,
        ))
        .expect("the call resolves");
    assert_eq!(prepared.url, "https://host");
}

#[test]
fn a_wrongly_typed_user_agent_is_a_400_naming_the_field() {
    let pat = key();
    let Err(error) = prepare(
        params(&[("user_agent", Json::Number(Number::from(5_i32)))]),
        Some(&pat),
        Some("https://host"),
    ) else {
        panic!("a number is not a user agent");
    };
    assert_eq!(error.status_code, 400);
    assert!(error.message.contains("user_agent"));
}

#[test]
fn a_missing_key_reaches_the_sdk_error() {
    let Err(error) = prepare(params(&[]), None, Some("https://host")) else {
        panic!("no key");
    };
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, SDK_CREDENTIALS);
}

#[rstest]
#[case::explicit_base(Some("https://given"), "https://given/embeddings")]
#[case::env_base(None, "https://from-env/embeddings")]
fn the_base_comes_from_the_argument_then_the_env(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let mut env = env();
    env.api_base = Some("https://from-env".to_owned());
    let pat = key();
    let runtime = runtime();
    let prepared = runtime
        .block_on(prepare_embedding(
            EmbeddingPreparation {
                optional_params: params(&[]),
                api_key: Some(&pat),
                api_base,
                custom_endpoint: false,
                headers: None,
            },
            &env,
            &NoTokens,
        ))
        .expect("the base resolves");
    assert_eq!(prepared.url, expected);
}
