use std::fs;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::lemonade_model_metadata::{
    DeploymentCredentials, LemonadeCredentials, ModelInfoClient, ModelInfoRequest,
};
use betterlite_models::provider_metadata_factory::adapters::{LemonadeSettings, Stop};
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, system_clock};
use betterlite_prompts::arguments::json::Json as ConfigJson;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use crate::self_hosted_dispatch::error::LemonadeError;
use crate::self_hosted_dispatch::lemonade::{
    PROVIDER, decode_model_names, get_config, get_models, transform_response,
};

use super::support::secret;

#[derive(Clone)]
enum Reply {
    Fetched(FetchResponse),
    Failed,
}

struct FakeClient {
    reply: Reply,
    requests: Mutex<Vec<ModelInfoRequest>>,
}

impl FakeClient {
    fn answering(status: u16, body: &str) -> Self {
        Self::raw(status, body.as_bytes())
    }

    fn raw(status: u16, body: &[u8]) -> Self {
        Self {
            reply: Reply::Fetched(FetchResponse::new(status, Vec::new(), body.to_vec())),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            reply: Reply::Failed,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelInfoRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelInfoClient for FakeClient {
    type Error = IoError;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let result = match &self.reply {
            Reply::Fetched(response) => Ok(response.clone()),
            Reply::Failed => Err(IoError::other("offline")),
        };
        ready(result)
    }
}

fn stored_credentials(api_base: Option<&str>, lemonade_key: Option<&str>) -> LemonadeCredentials {
    LemonadeCredentials {
        api_base: api_base.map(str::to_owned),
        api_key: None,
        lemonade_key: lemonade_key.map(secret),
    }
}

async fn list(
    client: &FakeClient,
    credentials: &LemonadeCredentials,
    deployment: DeploymentCredentials<'_>,
) -> Result<Vec<String>, LemonadeError> {
    get_models(client, credentials, deployment, Duration::from_secs(9)).await
}

fn every_field() -> LemonadeSettings {
    LemonadeSettings {
        repeat_penalty: Some(1.5_f64),
        functions: Some(vec![DecisionsJson::Text(String::from("f"))]),
        logit_bias: Some(IndexMap::from([(
            String::from("50256"),
            DecisionsJson::Number(Number::from(1_i32)),
        )])),
        max_completion_tokens: Some(20),
        max_tokens: Some(10),
        number: Some(2),
        presence_penalty: Some(1),
        stop: Some(Stop::Text(String::from("END"))),
        temperature: Some(1),
        top_p: Some(1),
        top_k: Some(5),
        response_format: Some(IndexMap::from([(
            String::from("type"),
            DecisionsJson::Text(String::from("json_object")),
        )])),
        tools: Some(vec![DecisionsJson::Text(String::from("tool"))]),
    }
}

#[test]
fn get_config_lists_only_the_configured_fields_in_the_class_body_order() {
    let config = get_config(&every_field()).expect("every configured field converts");
    let names: Vec<&str> = config.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "repeat_penalty",
            "functions",
            "logit_bias",
            "max_tokens",
            "max_completion_tokens",
            "n",
            "presence_penalty",
            "stop",
            "temperature",
            "top_p",
            "top_k",
            "response_format",
            "tools",
        ]
    );
}

#[test]
fn get_config_converts_the_configured_values() {
    let config = get_config(&every_field()).expect("every configured field converts");
    assert_eq!(
        config.get("stop"),
        Some(&ConfigJson::Text(String::from("END")))
    );
    assert_eq!(
        config.get("tools"),
        Some(&ConfigJson::List(vec![ConfigJson::Text(String::from(
            "tool"
        ))]))
    );
    assert_eq!(
        config.get("n"),
        Some(&ConfigJson::Number(Number::from(2_i32)))
    );
    assert_eq!(PROVIDER, "lemonade");
}

#[test]
fn get_config_skips_the_unset_fields() {
    let config = get_config(&LemonadeSettings::default()).expect("nothing to convert");
    assert!(config.is_empty());
    let converted = get_config(&LemonadeSettings {
        repeat_penalty: Some(0.5_f64),
        functions: None,
        logit_bias: None,
        max_completion_tokens: None,
        max_tokens: None,
        number: None,
        presence_penalty: None,
        stop: None,
        temperature: None,
        top_p: None,
        top_k: None,
        response_format: None,
        tools: None,
    })
    .expect("the set field converts");
    assert_eq!(converted.len(), 1);
    assert_eq!(
        converted.get("repeat_penalty"),
        Some(&ConfigJson::Number(
            Number::from_f64(0.5_f64).expect("a finite float")
        ))
    );
}

#[tokio::test]
async fn get_models_prefixes_every_id_and_targets_the_models_endpoint() {
    let client = FakeClient::answering(200, r#"{"data":[{"id":"a"},{"id":"b"}]}"#);
    let credentials = stored_credentials(Some("http://host:8000/api/v1"), None);
    let names = list(
        &client,
        &credentials,
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect("the reply lists two models");
    assert_eq!(names, ["lemonade/a", "lemonade/b"]);
    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://host:8000/api/v1/models");
    assert!(
        requests[0].headers.is_empty(),
        "the placeholder key sends no header"
    );
    assert_eq!(requests[0].timeout, Duration::from_secs(9));
}

#[tokio::test]
async fn get_models_sends_the_configured_key_and_keeps_the_default_base() {
    let client = FakeClient::answering(200, r#"{"data":[]}"#);
    let names = list(
        &client,
        &stored_credentials(None, Some("sk-lemon")),
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect("the empty list is fine");
    assert!(names.is_empty());
    let requests = client.requests();
    assert_eq!(requests[0].url, "http://localhost:8000/api/v1/models");
    let headers: Vec<(&str, String)> = requests[0]
        .headers
        .iter()
        .map(|(name, value)| (*name, value.expose_secret().to_owned()))
        .collect();
    assert_eq!(
        headers,
        [("Authorization", String::from("Bearer sk-lemon"))]
    );
}

#[tokio::test]
async fn get_models_keeps_the_placeholder_key_for_an_explicit_base_without_one() {
    let client = FakeClient::answering(200, r#"{"data":[]}"#);
    let credentials = LemonadeCredentials {
        api_base: None,
        api_key: None,
        lemonade_key: Some(secret("sk-lemon")),
    };
    list(
        &client,
        &credentials,
        DeploymentCredentials {
            api_base: Some("http://passed"),
            api_key: None,
        },
    )
    .await
    .expect("the empty list is fine");
    let requests = client.requests();
    assert_eq!(requests[0].url, "http://passed/models");
    assert!(
        requests[0].headers.is_empty(),
        "a passed base without its own key sends the placeholder key, which adds no header"
    );
}

#[tokio::test]
async fn get_models_reports_a_non_200_status_with_the_body() {
    let client = FakeClient::answering(500, "boom");
    let error = list(
        &client,
        &stored_credentials(Some("http://host"), None),
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect_err("a 500 fails the listing");
    assert_eq!(
        error.to_string(),
        "Failed to fetch models from Lemonade. Status code: 500, Response: boom"
    );
    assert!(matches!(error, LemonadeError::Status { status: 500, .. }));
}

#[tokio::test]
async fn get_models_refuses_a_status_body_that_is_not_utf8() {
    let client = FakeClient::raw(500, &[0xff, 0xfe]);
    let error = list(
        &client,
        &stored_credentials(Some("http://host"), None),
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect_err("a 500 fails the listing");
    assert_eq!(
        error.to_string(),
        "the 500 Lemonade models answer is not UTF-8 text: invalid utf-8 sequence of 1 bytes from index 0"
    );
    assert!(matches!(
        error,
        LemonadeError::StatusBody { status: 500, .. }
    ));
}

#[tokio::test]
async fn get_models_reports_a_transport_failure() {
    let client = FakeClient::failing();
    let error = list(
        &client,
        &stored_credentials(Some("http://host"), None),
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect_err("the transport fails");
    assert_eq!(
        error.to_string(),
        "Failed to fetch models from Lemonade. Set Lemonade API Base via `LEMONADE_API_BASE` environment variable. Error: offline"
    );
    assert!(matches!(error, LemonadeError::Request { .. }));
}

fn is_reply(error: &LemonadeError) -> bool {
    matches!(error, LemonadeError::Reply { .. })
}

fn is_shape(error: &LemonadeError) -> bool {
    matches!(error, LemonadeError::Shape)
}

fn is_entry(error: &LemonadeError) -> bool {
    matches!(error, LemonadeError::Entry)
}

#[rstest]
#[case::not_json("not json", is_reply, "the Lemonade models reply is not valid JSON: ")]
#[case::empty_list(
    "[]",
    is_shape,
    "the Lemonade models reply is not an object with a data list"
)]
#[case::data_object(
    r#"{"data":{}}"#,
    is_shape,
    "the Lemonade models reply is not an object with a data list"
)]
#[case::entry_without_id(
    r#"{"data":[{"name":"no-id"}]}"#,
    is_entry,
    "a Lemonade model entry holds no string id"
)]
#[case::entry_not_object(
    r#"{"data":["x"]}"#,
    is_entry,
    "a Lemonade model entry holds no string id"
)]
#[tokio::test]
async fn get_models_reports_replies_it_cannot_read(
    #[case] body: &str,
    #[case] variant: fn(&LemonadeError) -> bool,
    #[case] message: &str,
) {
    let client = FakeClient::answering(200, body);
    let error = list(
        &client,
        &stored_credentials(Some("http://host"), None),
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect_err("the reply cannot be read");
    assert!(variant(&error), "{body}");
    assert!(error.to_string().starts_with(message), "{body}");
}

#[tokio::test]
async fn get_models_lists_nothing_for_a_reply_without_data() {
    let client = FakeClient::answering(200, "{}");
    let names = list(
        &client,
        &stored_credentials(Some("http://host"), None),
        DeploymentCredentials {
            api_base: None,
            api_key: None,
        },
    )
    .await
    .expect("a reply without data lists nothing");
    assert!(names.is_empty());
}

// every corpus file goes through the models decoder the fuzzer feeds
#[test]
fn replays_the_lemonade_models_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/lemonade_models_reply")
        .expect("the models corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).expect("the corpus file reads");
            let _names = decode_model_names(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}

fn options() -> ReplyOptions<'static> {
    ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    }
}

const REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"other","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

#[test]
fn transform_response_names_the_requested_model() {
    let reply =
        transform_response(REPLY.as_bytes(), "requested", &options()).expect("the reply decodes");
    assert_eq!(reply.response.model, "lemonade/requested");
    assert!(reply.hidden_model.is_none());
}

#[test]
fn transform_response_reports_a_reply_it_cannot_parse() {
    let error = transform_response(b"not json", "requested", &options())
        .expect_err("the reply is not JSON");
    assert!(
        error
            .to_string()
            .starts_with("the Lemonade chat reply could not be read: ")
    );
    assert!(matches!(error, LemonadeError::ChatReply { .. }));
}
