#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankRequest, RerankResponse, RerankTokens,
};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{
    Captured, DiscardingSink, Factory, FixtureClient, Reply, Source, logging_state, now,
    prepost_env,
};
use crate::rerank::error::RerankCallError;
use crate::rerank::execute::{dispatch_request, resolve_target, transform_response};
use crate::rerank::plan::{LogContext, ParamMap, RerankCall, RerankEnvironment, prepare_request};
use crate::rerank::transport::RerankHttpRequest;

pub(super) const GENERATED_ID: &str = "00000000-0000-4000-8000-000000000000";
const MODEL_ARN: &str = "arn:aws:bedrock:us-east-1:123456789012:rerank-model/test";
const FIRST_ACCESS: &str = "AKIAIOSFODNN7EXAMPLE";
const FIRST_SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

pub(super) fn request(model: &str, query: &str, documents: Vec<RerankDocument>) -> RerankRequest {
    RerankRequest {
        model: model.to_owned(),
        query: query.to_owned(),
        top_n: None,
        documents,
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
        truncate_prompt_tokens: None,
        truncation_side: None,
        max_tokens_per_query: None,
    }
}

pub(super) fn text(value: &str) -> RerankDocument {
    RerankDocument::Text(value.to_owned())
}

pub(super) fn object(pairs: &[(&str, &str)]) -> RerankDocument {
    let mut fields = IndexMap::new();
    for (name, value) in pairs {
        fields.insert((*name).to_owned(), Json::Text((*value).to_owned()));
    }
    RerankDocument::Object(fields)
}

pub(super) fn params(pairs: &[(&str, Json)]) -> ParamMap {
    let mut map = ParamMap::new();
    for (key, value) in pairs {
        let encoded = serde_json::to_string(value).unwrap();
        map.insert(
            (*key).to_owned(),
            RawValue::from_string(encoded).expect("the param is valid JSON"),
        );
    }
    map
}

pub(super) fn auth_params() -> ParamMap {
    params(&[
        ("aws_access_key_id", Json::Text(FIRST_ACCESS.to_owned())),
        ("aws_secret_access_key", Json::Text(FIRST_SECRET.to_owned())),
        ("aws_region_name", Json::Text("us-east-1".to_owned())),
    ])
}

pub(super) fn call(request: &RerankRequest, optional_params: ParamMap) -> RerankCall<'_> {
    RerankCall {
        request,
        optional_params,
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        is_async: false,
        environment: RerankEnvironment::default(),
        now: now(),
    }
}

fn field<'json>(fields: &'json IndexMap<String, Json>, name: &str) -> &'json Json {
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the object carries `{name}`"))
}

fn object_field<'json>(
    fields: &'json IndexMap<String, Json>,
    name: &str,
) -> &'json IndexMap<String, Json> {
    let Json::Object(inner) = field(fields, name) else {
        panic!("`{name}` is an object");
    };
    inner
}

fn list_field<'json>(fields: &'json IndexMap<String, Json>, name: &str) -> &'json Vec<Json> {
    let Json::List(items) = field(fields, name) else {
        panic!("`{name}` is a list");
    };
    items
}

fn number_field(fields: &IndexMap<String, Json>, name: &str) -> i64 {
    let Json::Number(number) = field(fields, name) else {
        panic!("`{name}` is a number");
    };
    number.as_i64().expect("the number is an integer")
}

pub(super) struct Run {
    pub(super) result: Result<RerankResponse, RerankCallError>,
    pub(super) supplied_seen: Vec<(RerankHttpRequest, bool)>,
    pub(super) factory_seen: Vec<(RerankHttpRequest, bool)>,
    pub(super) sync_calls: u32,
    pub(super) async_calls: u32,
    pub(super) state: LoggingState,
    pub(super) captured: Captured,
}

impl Run {
    fn seen(&self) -> &Vec<(RerankHttpRequest, bool)> {
        if self.supplied_seen.is_empty() {
            &self.factory_seen
        } else {
            &self.supplied_seen
        }
    }

    pub(super) fn posted(&self) -> &RerankHttpRequest {
        let requests = self.seen();
        assert_eq!(requests.len(), 1, "one request reaches the provider");
        &requests[0].0
    }

    pub(super) fn with_logging(&self) -> bool {
        self.seen()[0].1
    }

    pub(super) fn headers(&self) -> BTreeMap<String, String> {
        self.posted().headers.iter().cloned().collect()
    }

    pub(super) fn body(&self) -> IndexMap<String, Json> {
        serde_json::from_slice(&self.posted().body).unwrap()
    }

    pub(super) fn text(&self) -> &str {
        str::from_utf8(&self.posted().body).unwrap()
    }

    pub(super) fn response(&self) -> &RerankResponse {
        self.result.as_ref().unwrap()
    }

    pub(super) fn pre_call(&self) -> Option<Json> {
        let details = self.captured.lock().unwrap().clone()?;
        let Json::Object(fields) = details else {
            return None;
        };
        fields.get("input").cloned()
    }

    pub(super) fn pre_call_args(&self) -> Option<Json> {
        let details = self.captured.lock().unwrap().clone()?;
        let Json::Object(fields) = details else {
            return None;
        };
        fields.get("additional_args").cloned()
    }

    pub(super) fn post_call(&self) -> Option<&Json> {
        self.state.model_call_details.get("original_response")
    }
}

struct Stages<'stage> {
    runtime: &'stage Runtime,
    source: &'stage Source,
    factory: &'stage Factory,
    redactor: &'stage Redactor,
}

pub(super) fn run_case(
    call: RerankCall<'_>,
    reply: Reply,
    supplied: Option<ClientKind>,
    redactor: &Redactor,
) -> Run {
    run_with_source(call, reply, supplied, &Source::new(), redactor)
}

pub(super) fn run_with_source(
    mut call: RerankCall<'_>,
    reply: Reply,
    supplied: Option<ClientKind>,
    source: &Source,
    redactor: &Redactor,
) -> Run {
    let client = FixtureClient::new(reply.clone());
    let factory = Factory::new(reply);
    let mut state = logging_state(redactor);
    let captured = Captured::default();
    let runtime = Builder::new_current_thread().build().unwrap();
    let supplied_ref = supplied.map(|kind| SuppliedClient {
        kind,
        transport: &client,
    });
    let stages = Stages {
        runtime: &runtime,
        source,
        factory: &factory,
        redactor,
    };
    let result = run_logged(&mut call, &stages, supplied_ref, &mut state, &captured);
    Run {
        result,
        supplied_seen: client.requests(),
        factory_seen: factory.requests(),
        sync_calls: *factory.sync_calls.lock().unwrap(),
        async_calls: *factory.async_calls.lock().unwrap(),
        state,
        captured,
    }
}

fn run_logged(
    call: &mut RerankCall<'_>,
    stages: &Stages<'_>,
    supplied: Option<SuppliedClient<'_, FixtureClient>>,
    state: &mut LoggingState,
    captured: &Captured,
) -> Result<RerankResponse, RerankCallError> {
    let sink = DiscardingSink;
    let logger = |details: &Json| -> Result<(), PrePostError> {
        let Json::Object(fields) = details else {
            return Ok(());
        };
        let pre_call = matches!(
            fields.get("log_event_type"),
            Some(Json::Text(event)) if event == "pre_api_call"
        );
        if pre_call {
            *captured.lock().unwrap() = Some(details.clone());
        }
        Ok(())
    };
    let env = prepost_env(state, stages.redactor, &sink, Some(&logger));
    let mut log = LogContext { state, env: &env };
    let target = stages
        .runtime
        .block_on(resolve_target(call, stages.source))?;
    let prepared = prepare_request(call, &target, &mut log)?;
    let reply = stages.runtime.block_on(dispatch_request(
        call,
        &prepared,
        supplied,
        stages.factory,
        log.state,
    ))?;
    transform_response(call, &reply, GENERATED_ID, &mut log)
}

const VALID_REPLY: &str = r#"{"id":"resp-1","results":[{"index":1,"relevanceScore":0.5},{"index":0,"relevanceScore":0.25}]}"#;

#[rstest]
#[case::text_and_json_document_source("text_and_json_document_source")]
#[case::rerank_model_arn("rerank_model_arn")]
#[case::top_n("top_n")]
#[case::results_index_score("results_index_score")]
#[case::return_document_content("return_document_content")]
#[case::rerank_error("rerank_error")]
#[case::rerank_transport_logging("rerank_transport_logging")]
fn rerank_cases(#[case] name: &str) {
    let redactor = super::fixture::redactor();
    match name {
        "text_and_json_document_source" => document_sources(&redactor),
        "rerank_model_arn" => model_arn(&redactor),
        "top_n" => top_n(&redactor),
        "results_index_score" => results_index_score(&redactor),
        "return_document_content" => return_document_content(&redactor),
        "rerank_error" => rerank_error(&redactor),
        "rerank_transport_logging" => transport_logging(&redactor),
        _ => panic!("unknown case {name}"),
    }
}

fn document_sources(redactor: &Redactor) {
    let mut documented = request(
        MODEL_ARN,
        "what is rust",
        vec![text("first"), object(&[("title", "second")])],
    );
    documented.top_n = Some(2);
    documented.rank_fields = Some(vec!["title".to_owned()]);
    documented.return_documents = Some(false);
    documented.max_chunks_per_doc = Some(4);
    let run = run_case(
        call(&documented, auth_params()),
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        redactor,
    );
    assert_eq!(
        run.text(),
        concat!(
            r#"{"queries": [{"textQuery": {"text": "what is rust"}, "type": "TEXT"}], "#,
            r#""rerankingConfiguration": {"bedrockRerankingConfiguration": {"modelConfiguration": "#,
            r#"{"modelArn": "arn:aws:bedrock:us-east-1:123456789012:rerank-model/test"}, "#,
            r#""numberOfResults": 2}, "type": "BEDROCK_RERANKING_MODEL"}, "#,
            r#""sources": [{"inlineDocumentSource": {"textDocument": {"text": "first"}, "type": "TEXT"}, "type": "INLINE"}, "#,
            r#"{"inlineDocumentSource": {"jsonDocument": {"title": "second"}, "type": "JSON"}, "type": "INLINE"}]}"#
        )
    );
    let body = run.body();
    assert_eq!(
        body.keys().cloned().collect::<Vec<_>>(),
        vec!["queries", "rerankingConfiguration", "sources"]
    );
    assert_source_shapes(&body);
}

fn assert_source_shapes(body: &IndexMap<String, Json>) {
    let sources = list_field(body, "sources");
    assert_eq!(sources.len(), 2);
    let Json::Object(first) = &sources[0] else {
        panic!("a source is an object");
    };
    assert_eq!(field(first, "type"), &Json::Text("INLINE".to_owned()));
    let inline = object_field(first, "inlineDocumentSource");
    assert_eq!(field(inline, "type"), &Json::Text("TEXT".to_owned()));
    let text_document = object_field(inline, "textDocument");
    assert_eq!(
        field(text_document, "text"),
        &Json::Text("first".to_owned())
    );
    let Json::Object(second) = &sources[1] else {
        panic!("a source is an object");
    };
    let inline_json = object_field(second, "inlineDocumentSource");
    assert_eq!(field(inline_json, "type"), &Json::Text("JSON".to_owned()));
    let document = object_field(inline_json, "jsonDocument");
    assert_eq!(field(document, "title"), &Json::Text("second".to_owned()));
}

fn model_arn(redactor: &Redactor) {
    model_arn_payload(redactor);
    model_arn_endpoint_urls(redactor);
    model_arn_environment_region(redactor);
    model_arn_bearer_api_key(redactor);
    model_arn_extra_headers(redactor);
}

fn model_arn_payload(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("doc")]);
    let mut prepared = call(&documented, auth_params());
    prepared.api_base = Some("https://bedrock-runtime.us-west-2.amazonaws.com");
    let run = run_case(
        prepared,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        redactor,
    );
    assert_eq!(
        run.posted().url,
        "https://bedrock-agent-runtime.us-west-2.amazonaws.com/rerank"
    );
    let body = run.body();
    let configuration = object_field(&body, "rerankingConfiguration");
    let bedrock = object_field(configuration, "bedrockRerankingConfiguration");
    let model = object_field(bedrock, "modelConfiguration");
    assert_eq!(field(model, "modelArn"), &Json::Text(MODEL_ARN.to_owned()));
    assert_eq!(
        field(configuration, "type"),
        &Json::Text("BEDROCK_RERANKING_MODEL".to_owned())
    );
    let headers = run.headers();
    let authorization = headers.get("Authorization").expect("the request is signed");
    assert!(authorization.starts_with("AWS4-HMAC-SHA256"));
    assert!(authorization.contains("/bedrock/aws4_request"));
    assert!(headers.contains_key("X-Amz-Date"));
}

fn model_arn_endpoint_urls(redactor: &Redactor) {
    let arn = request(MODEL_ARN, "q", vec![text("doc")]);
    let named = request("rerank-model", "q", vec![text("doc")]);
    let mut trailing = call(&arn, auth_params());
    trailing.api_base = Some("https://bedrock-runtime.us-west-2.amazonaws.com/");
    let configured = params(&[
        ("aws_access_key_id", Json::Text(FIRST_ACCESS.to_owned())),
        ("aws_secret_access_key", Json::Text(FIRST_SECRET.to_owned())),
        (
            "aws_bedrock_runtime_endpoint",
            Json::Text("https://bedrock-runtime.eu-west-1.amazonaws.com".to_owned()),
        ),
        ("aws_region_name", Json::Text("eu-west-1".to_owned())),
    ]);
    let mut session = call(&named, ParamMap::new());
    session.environment.session_region = Some("ap-south-1");
    let mut endpoint = call(&named, ParamMap::new());
    endpoint.environment.runtime_endpoint =
        Some("https://bedrock-runtime.ca-central-1.amazonaws.com");
    let sync = Some(ClientKind::Sync);
    for (prepared, url) in [
        (
            trailing,
            "https://bedrock-agent-runtime.us-west-2.amazonaws.com//rerank",
        ),
        (
            call(&arn, configured),
            "https://bedrock-agent-runtime.eu-west-1.amazonaws.com/rerank",
        ),
        (
            session,
            "https://bedrock-agent-runtime.ap-south-1.amazonaws.com/rerank",
        ),
        (
            endpoint,
            "https://bedrock-agent-runtime.ca-central-1.amazonaws.com/rerank",
        ),
    ] {
        let run = run_case(prepared, Reply::ok(VALID_REPLY), sync, redactor);
        assert_eq!(run.posted().url, url);
    }
}

fn model_arn_environment_region(redactor: &Redactor) {
    let documented = request("rerank-model", "q", vec![text("doc")]);
    let source = Source::new();
    let mut environment = call(&documented, ParamMap::new());
    environment.environment.region_name = Some("eu-central-1");
    environment.environment.region = Some("us-east-2");
    let run = run_with_source(
        environment,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        &source,
        redactor,
    );
    assert_eq!(
        run.posted().url,
        "https://bedrock-agent-runtime.eu-central-1.amazonaws.com/rerank"
    );
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "eu-central-1");
}

fn model_arn_bearer_api_key(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("doc")]);
    let token = secrecy::SecretString::from("bearer-token".to_owned());
    let mut bearer_like = call(&documented, auth_params());
    bearer_like.api_key = Some(&token);
    let run = run_case(
        bearer_like,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        redactor,
    );
    let authorization = &run.headers()["Authorization"];
    assert!(authorization.starts_with("AWS4-HMAC-SHA256"));
    assert!(!authorization.starts_with("Bearer"));
}

fn model_arn_extra_headers(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("doc")]);
    let headers = vec![
        (
            "Content-Type".to_owned(),
            "application/vnd.bedrock+json".to_owned(),
        ),
        (
            "Authorization".to_owned(),
            "Bearer explicit-token".to_owned(),
        ),
        ("x-custom".to_owned(), "value".to_owned()),
    ];
    let mut prepared = call(&documented, auth_params());
    prepared.extra_headers = Some(&headers);
    let run = run_case(
        prepared,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        redactor,
    );
    let posted = run.headers();
    assert_eq!(
        posted.get("Content-Type"),
        Some(&"application/vnd.bedrock+json".to_owned())
    );
    assert_eq!(
        posted.get("Authorization"),
        Some(&"Bearer explicit-token".to_owned())
    );
    assert_eq!(posted.get("x-custom"), Some(&"value".to_owned()));
}

fn top_n(redactor: &Redactor) {
    for (top_n, expected) in [(None, 3), (Some(0), 3), (Some(-2), -2), (Some(7), 7)] {
        let mut documented = request(MODEL_ARN, "q", vec![text("a"), text("b"), text("c")]);
        documented.top_n = top_n;
        let run = run_case(
            call(&documented, auth_params()),
            Reply::ok(VALID_REPLY),
            Some(ClientKind::Sync),
            redactor,
        );
        let body = run.body();
        let configuration = object_field(&body, "rerankingConfiguration");
        let bedrock = object_field(configuration, "bedrockRerankingConfiguration");
        assert_eq!(
            number_field(bedrock, "numberOfResults"),
            expected,
            "top_n {top_n:?}"
        );
        assert_eq!(
            list_field(&body, "sources").len(),
            3,
            "no document is truncated"
        );
        assert_eq!(run.response().results.as_ref().unwrap().len(), 2);
    }
}

fn billed_units(response: &RerankResponse) -> &RerankBilledUnits {
    let meta = response.meta.as_ref().expect("the reply carries meta");
    meta.billed_units
        .as_ref()
        .expect("billed units are present")
        .as_ref()
        .expect("billed units are not null")
}

fn tokens(response: &RerankResponse) -> &RerankTokens {
    let meta = response.meta.as_ref().expect("the reply carries meta");
    meta.tokens
        .as_ref()
        .expect("tokens are present")
        .as_ref()
        .expect("tokens are not null")
}

fn run_reply(body: &str, redactor: &Redactor) -> Run {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    run_case(
        call(&documented, auth_params()),
        Reply::ok(body),
        Some(ClientKind::Sync),
        redactor,
    )
}

fn results_index_score(redactor: &Redactor) {
    results_index_score_values(redactor);
    results_index_score_usage(redactor);
    results_index_score_usage_nulls(redactor);
    results_index_score_ids_and_failures(redactor);
}

fn results_index_score_values(redactor: &Redactor) {
    let run = run_reply(
        r#"{"id":"resp-1","results":[{"index":2,"relevanceScore":0.125},{"index":0,"relevanceScore":-0.5}]}"#,
        redactor,
    );
    let response = run.response();
    assert_eq!(response.id.as_deref(), Some("resp-1"));
    let results = response.results.as_ref().unwrap();
    assert_eq!(results[0].index, 2);
    assert_eq!(results[1].index, 0);
    assert!(results[0].relevance_score.0 > results[1].relevance_score.0);
    assert_eq!(billed_units(response).search_units, Some(Some(1)));
    assert_eq!(tokens(response).input_tokens, None);
}

fn results_index_score_usage(redactor: &Redactor) {
    let filled_run = run_reply(
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":{"search_units":5,"total_tokens":9,"input_tokens":1,"output_tokens":2}}"#,
        redactor,
    );
    let response = filled_run.response();
    assert_eq!(billed_units(response).search_units, Some(Some(5)));
    assert_eq!(billed_units(response).total_tokens, Some(Some(9)));
    assert_eq!(tokens(response).input_tokens, Some(Some(1)));
    assert_eq!(tokens(response).output_tokens, Some(Some(2)));

    let empty_run = run_reply(
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":{}}"#,
        redactor,
    );
    assert_eq!(
        billed_units(empty_run.response()).search_units,
        None,
        "an empty usage keeps no search unit"
    );

    let null_run = run_reply(
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":null}"#,
        redactor,
    );
    assert_eq!(
        billed_units(null_run.response()).search_units,
        Some(Some(1)),
        "a null usage takes the default of one search unit"
    );
}

fn results_index_score_usage_nulls(redactor: &Redactor) {
    let run = run_reply(
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":{"search_units":null,"total_tokens":null,"input_tokens":null,"output_tokens":null}}"#,
        redactor,
    );
    let response = run.response();
    assert_eq!(billed_units(response).search_units, Some(None));
    assert_eq!(billed_units(response).total_tokens, Some(None));
    assert_eq!(tokens(response).input_tokens, Some(None));
    assert_eq!(tokens(response).output_tokens, Some(None));
}

const DECODE_OUTCOMES: [(&str, Option<&str>); 20] = [
    (
        r#"{"results":[{"index":0,"relevanceScore":0.5}]}"#,
        Some(GENERATED_ID),
    ),
    (
        r#"{"id":"explicit","results":[{"index":0,"relevanceScore":0.5}]}"#,
        Some("explicit"),
    ),
    (
        r#"{"id":"","results":[{"index":0,"relevanceScore":0.5}]}"#,
        Some(""),
    ),
    (
        r#"{"id":null,"results":[{"index":0,"relevanceScore":0.5}]}"#,
        Some(GENERATED_ID),
    ),
    (r#"{"usage":{}}"#, None),
    ("[]", None),
    (r#"{"results":[]}"#, None),
    (r#"{"results":null}"#, None),
    (r#"{"results":{"index":0}}"#, None),
    (r#"{"results":[5]}"#, None),
    (r#"{"results":[{"relevanceScore":0.5}]}"#, None),
    (r#"{"results":[{"index":null,"relevanceScore":0.5}]}"#, None),
    (r#"{"results":[{"index":1.5,"relevanceScore":0.5}]}"#, None),
    (r#"{"results":[{"index":0,"relevanceScore":"half"}]}"#, None),
    (r#"{"results":[{"index":0,"relevanceScore":null}]}"#, None),
    (
        r#"{"results":[{"index":0,"relevanceScore":0.12345678901234567890123456789012}]}"#,
        None,
    ),
    (
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":"none"}"#,
        None,
    ),
    (
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":{"search_units":1.5}}"#,
        None,
    ),
    (
        r#"{"results":[{"index":0,"relevanceScore":0.5}],"usage":{"total_tokens":"many"}}"#,
        None,
    ),
    (
        r#"{"id":7,"results":[{"index":0,"relevanceScore":0.5}]}"#,
        None,
    ),
];

fn results_index_score_ids_and_failures(redactor: &Redactor) {
    for (body, id) in DECODE_OUTCOMES {
        let run = run_reply(body, redactor);
        match id {
            Some(expected) => {
                assert_eq!(run.response().id.as_deref(), Some(expected), "{body}");
            }
            None => assert!(
                matches!(run.result, Err(RerankCallError::Decode(_))),
                "{body}"
            ),
        }
    }
}

fn return_document_content(redactor: &Redactor) {
    let documents = vec![text("a"), text("b")];
    let mut bodies = Vec::new();
    for (return_documents, rank_fields, max_chunks) in [
        (None, None, None),
        (Some(true), Some(vec!["title".to_owned()]), Some(2)),
        (Some(false), None, Some(0)),
    ] {
        let mut documented = request(MODEL_ARN, "q", documents.clone());
        documented.return_documents = return_documents;
        documented.rank_fields = rank_fields;
        documented.max_chunks_per_doc = max_chunks;
        let run = run_case(
            call(&documented, auth_params()),
            Reply::ok(VALID_REPLY),
            Some(ClientKind::Sync),
            redactor,
        );
        bodies.push(run.text().to_owned());
        assert!(
            run.response()
                .results
                .as_ref()
                .unwrap()
                .iter()
                .all(|result| result.document.is_none()),
            "no source document is attached to a result"
        );
    }
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[1], bodies[2]);
}

fn rerank_error(redactor: &Redactor) {
    rerank_error_status_and_transport(redactor);
    rerank_error_redirect(redactor);
    rerank_error_decode(redactor);
}

fn rerank_error_status_and_transport(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    let sync = Some(ClientKind::Sync);
    for (reply, code, message, headers) in [
        (
            Reply::status(
                429,
                &[("x-amzn-errortype", "ThrottlingException")],
                "slow down",
            ),
            429,
            "slow down",
            vec![(
                "x-amzn-errortype".to_owned(),
                "ThrottlingException".to_owned(),
            )],
        ),
        (Reply::Timeout, 408, "Timeout error occurred.", Vec::new()),
    ] {
        let run = run_case(call(&documented, auth_params()), reply, sync, redactor);
        let Err(RerankCallError::Provider { status, error }) = &run.result else {
            panic!("a status error or a timeout is a provider error");
        };
        assert_eq!(*status, code);
        assert_eq!(run.result.as_ref().unwrap_err().status(), Some(code));
        assert_eq!(error.error.status_code, code);
        assert_eq!(error.error.message, message);
        assert_eq!(error.response_headers, headers);
        assert!(run.post_call().is_none(), "a failed call logs no post_call");
    }

    let failed_run = run_case(
        call(&documented, auth_params()),
        Reply::Failed,
        sync,
        redactor,
    );
    assert!(matches!(
        failed_run.result,
        Err(RerankCallError::Transport(_))
    ));
    assert_eq!(failed_run.result.as_ref().unwrap_err().status(), None);
}

fn rerank_error_redirect(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    let sync = Some(ClientKind::Sync);
    let redirect_run = run_case(
        call(&documented, auth_params()),
        Reply::status(302, &[], VALID_REPLY),
        sync,
        redactor,
    );
    let Err(RerankCallError::Provider {
        status: redirect_status,
        ..
    }) = &redirect_run.result
    else {
        panic!("raise_for_status raises for a redirect");
    };
    assert_eq!(*redirect_status, 302);
    assert!(
        redirect_run.post_call().is_none(),
        "a redirect logs no post_call of the body it did not decode"
    );
}

fn rerank_error_decode(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    let sync_run = run_case(
        call(&documented, auth_params()),
        Reply::ok("{not json"),
        Some(ClientKind::Sync),
        redactor,
    );
    assert!(matches!(sync_run.result, Err(RerankCallError::Decode(_))));
    assert!(
        sync_run.post_call().is_some(),
        "the sync path logs the text before decoding it"
    );

    let mut asynchronous = call(&documented, auth_params());
    asynchronous.is_async = true;
    let async_run = run_case(
        asynchronous,
        Reply::ok("{not json"),
        Some(ClientKind::Asynchronous),
        redactor,
    );
    assert!(matches!(async_run.result, Err(RerankCallError::Decode(_))));
    assert!(
        async_run.post_call().is_none(),
        "the async path has no post_call"
    );
}

fn transport_logging(redactor: &Redactor) {
    transport_logging_sync(redactor);
    transport_logging_async(redactor);
    transport_logging_fallback(redactor);
}

fn transport_logging_sync(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    let mut synchronous = call(&documented, auth_params());
    synchronous.timeout = Some(Duration::from_millis(2500));
    let run = run_case(
        synchronous,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        redactor,
    );
    assert_eq!(run.supplied_seen.len(), 1);
    assert!(!run.with_logging());
    assert_eq!(run.sync_calls, 0);
    assert_eq!(run.posted().timeout, Some(Duration::from_millis(2500)));
    assert_eq!(
        run.pre_call(),
        Some(Json::Object(run.body())),
        "pre_call receives the request body"
    );
    let arguments = run.pre_call_args().expect("pre_call records its arguments");
    let Json::Object(recorded) = arguments else {
        panic!("pre_call records an object");
    };
    assert_eq!(
        recorded["api_base"],
        Json::Text("https://bedrock-agent-runtime.us-east-1.amazonaws.com/rerank".to_owned())
    );
    assert_eq!(recorded["complete_input_dict"], Json::Object(run.body()));
    let Json::Object(headers) = &recorded["headers"] else {
        panic!("pre_call records the prepared headers");
    };
    assert_eq!(
        headers["Content-Type"],
        Json::Text("application/json".to_owned())
    );
    assert!(
        headers.contains_key("Authorization"),
        "the prepared headers are the signed ones"
    );
    assert!(run.post_call().is_some());
}

fn transport_logging_async(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    let mut asynchronous = call(&documented, auth_params());
    asynchronous.is_async = true;
    let run = run_case(
        asynchronous,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Asynchronous),
        redactor,
    );
    assert_eq!(run.supplied_seen.len(), 1);
    assert!(
        run.with_logging(),
        "the async client receives the logging object"
    );
    assert_eq!(run.async_calls, 0);
    assert!(run.post_call().is_none());
}

fn transport_logging_fallback(redactor: &Redactor) {
    let documented = request(MODEL_ARN, "q", vec![text("a")]);
    let mut async_mismatch = call(&documented, auth_params());
    async_mismatch.is_async = true;
    let async_fallback_run = run_case(
        async_mismatch,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Sync),
        redactor,
    );
    assert!(async_fallback_run.supplied_seen.is_empty());
    assert_eq!(async_fallback_run.async_calls, 1);
    assert!(async_fallback_run.with_logging());

    let mut sync_mismatch = call(&documented, auth_params());
    sync_mismatch.is_async = false;
    let sync_fallback_run = run_case(
        sync_mismatch,
        Reply::ok(VALID_REPLY),
        Some(ClientKind::Asynchronous),
        redactor,
    );
    assert!(sync_fallback_run.supplied_seen.is_empty());
    assert_eq!(sync_fallback_run.sync_calls, 1);
    assert!(!sync_fallback_run.with_logging());

    let default_run = run_case(
        call(&documented, auth_params()),
        Reply::ok(VALID_REPLY),
        None,
        redactor,
    );
    assert_eq!(default_run.sync_calls, 1);
    assert!(!default_run.with_logging());
}
