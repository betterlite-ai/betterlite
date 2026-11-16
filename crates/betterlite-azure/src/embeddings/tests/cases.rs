use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use crate::embeddings::client::Fields;
use crate::embeddings::logging::ResponseKind;
use crate::endpoints::projection::ClientKind;

use super::support::{Answer, Calls, Recorder, Run, call, headers, json_reply, run, supplied};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u32) -> Json {
    Json::Number(Number::from(value))
}

fn fields(pairs: Vec<(&str, Json)>) -> Fields {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn sent(input: &EmbeddingInput, aembedding: bool, answer: &Answer) -> Run {
    run(
        call(input, aembedding),
        answer,
        &Calls::default(),
        Recorder::default(),
    )
}

fn assert_input(input: &EmbeddingInput, expected: &Json) {
    let outcome = sent(input, false, &json_reply(&[]));
    assert!(outcome.result.is_ok());
    let (data, timeout) = &outcome.calls[0];
    assert_eq!(data.keys().collect::<Vec<_>>(), ["model", "input"]);
    assert_eq!(data["model"], text("my-deployment"));
    assert_eq!(&data["input"], expected);
    assert_eq!(timeout.to_bits(), 30.0_f64.to_bits());
    let pre = &outcome.recorder.pre[0];
    assert_eq!(&pre.input, expected);
    assert_eq!(&pre.complete_input_dict, data);
    assert_eq!(pre.api_key.as_ref().unwrap().expose_secret(), "azure-key");
    assert!(pre.azure_ad_token.is_none());
}

pub(super) fn string_and_token_input() {
    assert_input(&EmbeddingInput::Text("hello".to_owned()), &text("hello"));
    assert_input(
        &EmbeddingInput::Tokens(vec![1, 2, 3]),
        &Json::List(vec![number(1), number(2), number(3)]),
    );
    assert_input(
        &EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]]),
        &Json::List(vec![
            Json::List(vec![number(1), number(2)]),
            Json::List(vec![number(3)]),
        ]),
    );
}

pub(super) fn list_input() {
    let input = EmbeddingInput::Texts(vec!["first".to_owned(), "second".to_owned()]);
    let outcome = sent(&input, false, &json_reply(&[]));
    let (data, _timeout) = &outcome.calls[0];
    assert_eq!(
        data["input"],
        Json::List(vec![text("first"), text("second")])
    );
    let reply = outcome.result.unwrap().unwrap();
    assert_eq!(reply.model, "text-embedding-3-small");
    assert_eq!(reply.object, "list");
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.data[0].index, 0);
    let post = &outcome.recorder.post[0];
    assert_eq!(post.kind, ResponseKind::SdkObject);
    assert_eq!(post.input, data["input"]);
    let Json::Object(original) = &post.original_response else {
        panic!("the post call records the response object");
    };
    assert_eq!(original["model"], text("text-embedding-3-small"));
    assert_eq!(&post.complete_input_dict, data);
    assert_eq!(
        post.api_base.as_deref(),
        Some("https://example.openai.azure.com")
    );
}

fn with_params(params: Fields, extra_headers: &[(&str, &str)]) -> Fields {
    let input = EmbeddingInput::Text("a".to_owned());
    let mut request = call(&input, false);
    request.optional_params = params;
    request.headers = headers(extra_headers);
    let outcome = run(
        request,
        &json_reply(&[]),
        &Calls::default(),
        Recorder::default(),
    );
    outcome.calls[0].0.clone()
}

pub(super) fn dimensions_encoding() {
    let retained = with_params(
        fields(vec![
            ("dimensions", number(256)),
            ("encoding_format", text("float")),
            ("user", text("u")),
        ]),
        &[],
    );
    assert_eq!(
        retained.keys().collect::<Vec<_>>(),
        ["model", "input", "dimensions", "encoding_format", "user"]
    );
    assert_eq!(retained["dimensions"], number(256));
    assert_eq!(retained["encoding_format"], text("float"));
    let overridden = with_params(
        fields(vec![("input", text("x")), ("model", text("other"))]),
        &[],
    );
    assert_eq!(overridden.keys().collect::<Vec<_>>(), ["model", "input"]);
    assert_eq!(overridden["model"], text("other"));
    assert_eq!(overridden["input"], text("x"));
    extra_headers();
    client_settings();
}

fn extra_headers() {
    let existing = || {
        fields(vec![(
            "extra_headers",
            Json::Object(fields(vec![("a", text("b"))])),
        )])
    };
    let replaced = with_params(existing(), &[("x-h", "v")]);
    assert_eq!(
        replaced["extra_headers"],
        Json::Object(fields(vec![("x-h", text("v"))]))
    );
    let kept = with_params(existing(), &[]);
    assert_eq!(kept["extra_headers"], existing()["extra_headers"]);
    assert!(!with_params(Fields::new(), &[]).contains_key("extra_headers"));
}

fn client_settings() {
    let input = EmbeddingInput::Text("a".to_owned());
    let mut request = call(&input, false);
    request.litellm_params.max_retries = Some(7);
    request.litellm_params.timeout = Some(2.5);
    let outcome = run(
        request,
        &json_reply(&[]),
        &Calls::default(),
        Recorder::default(),
    );
    assert_eq!(outcome.built[0].max_retries, Some(7));
    assert_eq!(outcome.built[0].timeout, Some(2.5));
    let defaulted = sent(&input, false, &json_reply(&[]));
    assert_eq!(defaulted.built[0].max_retries, Some(2));
    assert_eq!(defaulted.built[0].timeout, None);
    let mut standalone = call(&input, false);
    standalone.max_retries = Some(9);
    let ignored = run(
        standalone,
        &json_reply(&[]),
        &Calls::default(),
        Recorder::default(),
    );
    assert_eq!(ignored.built[0].max_retries, Some(2));
}

fn built_kind(version: Option<&str>, aembedding: bool) -> (ClientKind, Option<String>) {
    let input = EmbeddingInput::Text("a".to_owned());
    let mut request = call(&input, aembedding);
    request.api_version = version;
    let outcome = run(
        request,
        &json_reply(&[]),
        &Calls::default(),
        Recorder::default(),
    );
    assert!(outcome.result.is_ok());
    (outcome.built[0].kind, outcome.built[0].endpoint.clone())
}

pub(super) fn deployment_and_v1_path() {
    let azure = Some("https://example.openai.azure.com".to_owned());
    assert_eq!(
        built_kind(Some("2024-02-01"), false),
        (ClientKind::AzureOpenAi, azure.clone())
    );
    assert_eq!(
        built_kind(Some("2024-02-01"), true),
        (ClientKind::AsyncAzureOpenAi, azure)
    );
    let v1 = Some("https://example.openai.azure.com/openai/v1/".to_owned());
    assert_eq!(
        built_kind(Some("v1"), false),
        (ClientKind::OpenAi, v1.clone())
    );
    assert_eq!(
        built_kind(Some("preview"), true),
        (ClientKind::AsyncOpenAi, v1.clone())
    );
    assert_eq!(built_kind(Some("latest"), false), (ClientKind::OpenAi, v1));
    assert_eq!(built_kind(Some("V1"), false).0, ClientKind::AzureOpenAi);
    assert_eq!(built_kind(None, false).0, ClientKind::AzureOpenAi);
    supplied_client_skips_the_factory();
}

fn supplied_client_skips_the_factory() {
    let input = EmbeddingInput::Text("a".to_owned());
    let answer = json_reply(&[]);
    let calls = Calls::default();
    let client = supplied(ClientKind::OpenAi, &answer, &calls);
    let mut request = call(&input, false);
    request.client = Some(&client);
    let outcome = run(request, &answer, &calls, Recorder::default());
    assert!(outcome.result.is_ok());
    assert!(outcome.built.is_empty());
    assert_eq!(outcome.calls.len(), 1);
}
