#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::Number;

use super::decode::{
    AgentOperation, AgentPayload, RESPONSE_BODY_LIMIT, RawResponse, decode_studio_agent_delete,
    decode_studio_agent_payload,
};
use super::error::StudioAgentError;
use super::headers::{AgentHeaders, validate_environment};
use super::records::{AgentDeleteResult, AgentPage, AgentWarning, StudioAgent};
use super::request::{AgentParams, BaseEnvironment, create_agent_body};
use super::urls::{collection_target, item_target, versions_target};

const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings(base: &str, key: Option<&str>) -> Settings {
    Settings {
        environment_key: key.map(secret),
        generic_api_key: None,
        api_base: base.to_owned(),
    }
}

fn caller() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn base_params() -> AgentParams {
    AgentParams {
        api_key: None,
        api_base: None,
        base_agent: None,
        instructions: None,
        base_environment: None,
        page_size: None,
        page_token: None,
    }
}

fn key_of(headers: &AgentHeaders) -> String {
    headers["x-goog-api-key"].expose_secret().to_owned()
}

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn dumped<T>(value: &T) -> Json
where
    T: Serialize,
{
    parse(&serde_json::to_string(value).unwrap())
}

fn reply<'raw>(
    status: u16,
    headers: &'raw IndexMap<String, String>,
    body: &'raw str,
) -> RawResponse<'raw> {
    RawResponse {
        status,
        headers,
        body: body.as_bytes(),
    }
}

fn agent_of(operation: AgentOperation, body: &str) -> (StudioAgent, Vec<AgentWarning>) {
    let headers = IndexMap::new();
    let decoded =
        decode_studio_agent_payload(&reply(200, &headers, body), operation, "req").unwrap();
    let AgentPayload::Agent(agent) = decoded.payload else {
        panic!("this reply is an agent record");
    };
    (agent, decoded.warnings)
}

fn page_of(operation: AgentOperation, body: &str) -> (Vec<IndexMap<String, Json>>, Option<String>) {
    let headers = IndexMap::new();
    let decoded =
        decode_studio_agent_payload(&reply(200, &headers, body), operation, "req").unwrap();
    match decoded.payload {
        AgentPayload::Agents(page) => (page.agents, page.next_page_token),
        AgentPayload::Versions(page) => (page.agent_versions, page.next_page_token),
        AgentPayload::Agent(_) => panic!("this reply is a page"),
    }
}

#[test]
fn collection_and_item_urls_follow_the_resolved_base() {
    let params = base_params();
    assert_eq!(
        collection_target(&params, &settings(DEFAULT_BASE, None)).url,
        "https://generativelanguage.googleapis.com/v1beta/agents"
    );
    let mut custom = base_params();
    custom.api_base = Some("https://custom.example".to_owned());
    assert_eq!(
        collection_target(&custom, &settings(DEFAULT_BASE, None)).url,
        "https://custom.example/v1beta/agents"
    );
    let slashed = settings("https://env.example/", None);
    assert_eq!(
        item_target("my agent", &params, &slashed).url,
        "https://env.example//v1beta/agents/my agent"
    );
    assert_eq!(
        versions_target("my/agent", &params, &slashed).url,
        "https://env.example//v1beta/agents/my/agent/versions"
    );
    let mut empty = base_params();
    empty.api_base = Some(String::new());
    assert_eq!(
        collection_target(&empty, &settings(DEFAULT_BASE, None)).url,
        "https://generativelanguage.googleapis.com/v1beta/agents"
    );
}

#[test]
fn pagination_includes_page_size_and_token_only_when_set() {
    let mut params = base_params();
    params.page_size = Some(5);
    params.page_token = Some("token".to_owned());
    assert_eq!(
        collection_target(&params, &settings(DEFAULT_BASE, None)).query,
        vec![
            ("pageSize".to_owned(), "5".to_owned()),
            ("pageToken".to_owned(), "token".to_owned()),
        ]
    );
    assert_eq!(
        versions_target("a", &params, &settings(DEFAULT_BASE, None)).query,
        vec![
            ("pageSize".to_owned(), "5".to_owned()),
            ("pageToken".to_owned(), "token".to_owned()),
        ]
    );
    params.page_size = Some(0);
    params.page_token = Some(String::new());
    assert!(
        collection_target(&params, &settings(DEFAULT_BASE, None))
            .query
            .is_empty()
    );
    assert!(
        versions_target("a", &params, &settings(DEFAULT_BASE, None))
            .query
            .is_empty()
    );
    params.page_size = None;
    params.page_token = None;
    assert!(
        collection_target(&params, &settings(DEFAULT_BASE, None))
            .query
            .is_empty()
    );
}

#[test]
fn validate_environment_copies_caller_headers_and_forces_json() {
    let headers = [
        ("X-Extra".to_owned(), "v".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
        ("x-goog-api-key".to_owned(), "caller".to_owned()),
    ]
    .into_iter()
    .collect();
    let mut params = base_params();
    params.api_key = Some(secret("explicit"));
    let validated =
        validate_environment(&headers, &params, &settings(DEFAULT_BASE, Some("env"))).unwrap();
    assert_eq!(key_of(&validated), "explicit");
    assert_eq!(validated["X-Extra"].expose_secret(), "v");
    assert_eq!(
        validated["Content-Type"].expose_secret(),
        "application/json"
    );
}

#[rstest]
#[case::explicit_wins(Some("explicit"), "explicit")]
#[case::empty_explicit_is_unset(Some(""), "env")]
#[case::environment_key_alone(None, "env")]
fn key_precedence_is_explicit_then_environment(
    #[case] explicit: Option<&str>,
    #[case] expected: &str,
) {
    let mut params = base_params();
    params.api_key = explicit.map(secret);
    let headers =
        validate_environment(&caller(), &params, &settings(DEFAULT_BASE, Some("env"))).unwrap();
    assert_eq!(key_of(&headers), expected);
}

#[test]
fn a_custom_base_is_refused_without_an_explicit_key() {
    let mut params = base_params();
    params.api_base = Some("https://custom.example".to_owned());
    let failure = validate_environment(
        &caller(),
        &params,
        &settings(DEFAULT_BASE, Some("env-google")),
    )
    .expect_err("the environment key must not reach a custom base");
    assert!(matches!(failure, StudioAgentError::CustomBaseWithoutKey));
    let message = failure.to_string();
    assert!(!message.contains("env-google"), "{message}");
}

#[test]
fn a_custom_base_accepts_an_explicit_key() {
    let mut params = base_params();
    params.api_base = Some("https://custom.example".to_owned());
    params.api_key = Some(secret("explicit"));
    let headers = validate_environment(&caller(), &params, &settings(DEFAULT_BASE, None)).unwrap();
    assert_eq!(key_of(&headers), "explicit");
}

#[test]
fn an_empty_api_base_still_uses_the_environment_key() {
    let mut params = base_params();
    params.api_base = Some(String::new());
    let headers =
        validate_environment(&caller(), &params, &settings(DEFAULT_BASE, Some("env"))).unwrap();
    assert_eq!(key_of(&headers), "env");
}

#[test]
fn key_context_isolation() {
    let shared = settings(DEFAULT_BASE, Some("env-google"));
    let mut explicit = base_params();
    explicit.api_key = Some(secret("explicit"));
    assert_eq!(
        key_of(&validate_environment(&caller(), &explicit, &shared).unwrap()),
        "explicit"
    );
    let missing = validate_environment(&caller(), &base_params(), &shared).unwrap();
    assert_eq!(key_of(&missing), "env-google");
    let mut custom = base_params();
    custom.api_base = Some("https://custom.example".to_owned());
    let refused = validate_environment(&caller(), &custom, &shared);
    assert!(matches!(
        refused,
        Err(StudioAgentError::CustomBaseWithoutKey)
    ));
}

#[test]
fn missing_keys_are_a_value_error() {
    let failure = validate_environment(&caller(), &base_params(), &settings(DEFAULT_BASE, None))
        .expect_err("no key is available");
    assert!(matches!(failure, StudioAgentError::MissingApiKey));
}

#[test]
fn create_body_forwards_only_the_three_native_values() {
    let mut params = base_params();
    params.base_agent = Some("waverunner".to_owned());
    params.instructions = Some("be nice".to_owned());
    params.base_environment = Some(BaseEnvironment::Object(IndexMap::from([(
        "type".to_owned(),
        Json::Text("remote".to_owned()),
    )])));
    params.api_key = Some(secret("explicit"));
    params.page_size = Some(5);
    assert_eq!(
        dumped(&create_agent_body("my-agent", &params)),
        parse(
            r#"{"name":"my-agent","base_agent":"waverunner","instructions":"be nice",
                "base_environment":{"type":"remote"}}"#
        )
    );
    assert_eq!(
        dumped(&create_agent_body("my-agent", &base_params())),
        parse(r#"{"name":"my-agent"}"#)
    );
}

#[test]
fn empty_native_values_survive_in_the_create_body() {
    let mut params = base_params();
    params.base_agent = Some(String::new());
    params.instructions = Some(String::new());
    params.base_environment = Some(BaseEnvironment::Object(IndexMap::new()));
    assert_eq!(
        dumped(&create_agent_body("n", &params)),
        parse(r#"{"name":"n","base_agent":"","instructions":"","base_environment":{}}"#)
    );
    params.base_environment = Some(BaseEnvironment::Id("env-1".to_owned()));
    assert_eq!(
        dumped(&create_agent_body("n", &params)),
        parse(r#"{"name":"n","base_agent":"","instructions":"","base_environment":"env-1"}"#)
    );
}

#[test]
fn created_agents_keep_provider_fields_beside_the_identity() {
    let (agent, warnings) = agent_of(
        AgentOperation::Create,
        r#"{"id":"i","name":"n","base_agent":"waverunner","z":[1]}"#,
    );
    assert_eq!(agent.id.as_deref(), Some("i"));
    assert_eq!(agent.name.as_deref(), Some("n"));
    assert_eq!(agent.extension.len(), 2);
    assert_eq!(
        agent.extension["z"],
        Json::List(vec![Json::Number(Number::from(1_i64))])
    );
    assert!(warnings.is_empty());
    assert_eq!(
        dumped(&agent),
        parse(r#"{"id":"i","name":"n","base_agent":"waverunner","z":[1]}"#)
    );
}

#[test]
fn absent_identity_defaults_to_the_request_name() {
    let (agent, _) = agent_of(AgentOperation::Get, "{}");
    assert_eq!(agent.id.as_deref(), Some("req"));
    assert_eq!(agent.name.as_deref(), Some("req"));
    assert!(agent.extension.is_empty());
}

#[test]
fn a_present_null_identity_is_not_replaced() {
    let (agent, _) = agent_of(AgentOperation::Create, r#"{"id":null}"#);
    assert_eq!(agent.id, None);
    assert_eq!(agent.name, None);
    let (named, _) = agent_of(AgentOperation::Get, r#"{"name":null}"#);
    assert_eq!(named.id.as_deref(), Some("req"));
    assert_eq!(named.name, None);
}

#[test]
fn a_missing_id_takes_the_request_name_and_keeps_the_provider_name() {
    let (agent, _) = agent_of(AgentOperation::Create, r#"{"name":"provider"}"#);
    assert_eq!(agent.id.as_deref(), Some("req"));
    assert_eq!(agent.name.as_deref(), Some("provider"));
    let (empty, _) = agent_of(AgentOperation::Create, r#"{"id":""}"#);
    assert_eq!(empty.id.as_deref(), Some(""));
    assert_eq!(empty.name.as_deref(), Some(""));
}

#[test]
fn a_malformed_create_reply_warns_and_falls_back() {
    let (agent, warnings) = agent_of(AgentOperation::Create, "not json");
    assert_eq!(agent.id.as_deref(), Some("req"));
    assert_eq!(agent.name.as_deref(), Some("req"));
    assert_eq!(warnings, vec![AgentWarning::MalformedCreateReply]);
    assert_eq!(
        serde_json::to_value(&warnings).expect("warnings serialize"),
        serde_json::json!(["malformed_create_reply"])
    );
}

#[test]
fn a_malformed_get_reply_falls_back_without_a_warning() {
    let (agent, warnings) = agent_of(AgentOperation::Get, "not json");
    assert_eq!(agent.id.as_deref(), Some("req"));
    assert!(warnings.is_empty());
}

#[rstest]
#[case::array("[]")]
#[case::null("null")]
#[case::text(r#""text""#)]
fn valid_json_of_the_wrong_shape_still_fails(#[case] body: &str) {
    let headers = IndexMap::new();
    let failure =
        decode_studio_agent_payload(&reply(200, &headers, body), AgentOperation::Create, "req")
            .expect_err("a non-object reply fails");
    assert!(matches!(failure, StudioAgentError::Shape { field: "root" }));
}

#[rstest]
#[case::id(r#"{"id":5}"#, "id")]
#[case::name(r#"{"name":true}"#, "name")]
fn a_non_text_identity_names_the_field(#[case] body: &str, #[case] field: &str) {
    let headers = IndexMap::new();
    let failure =
        decode_studio_agent_payload(&reply(200, &headers, body), AgentOperation::Get, "req")
            .expect_err("a non-text identity fails");
    assert!(matches!(failure, StudioAgentError::Shape { field: found } if found == field));
}

#[test]
fn status_before_decode() {
    let mut headers = IndexMap::new();
    headers.insert("x-goog-request".to_owned(), "r".to_owned());
    let valid = r#"{"id":"i","name":"n"}"#;
    let failure =
        decode_studio_agent_payload(&reply(500, &headers, valid), AgentOperation::Get, "req")
            .expect_err("a 500 is a failure before the body is decoded");
    let StudioAgentError::Gemini {
        status,
        body,
        headers: carried,
    } = failure
    else {
        panic!("expected a Gemini error");
    };
    assert_eq!((status, body.as_str(), &carried), (500, valid, &headers));
    let garbage = decode_studio_agent_payload(
        &reply(404, &headers, "not json"),
        AgentOperation::Create,
        "req",
    );
    assert!(matches!(
        garbage,
        Err(StudioAgentError::Gemini { status: 404, .. })
    ));
}

#[test]
fn a_non_2xx_delete_is_a_gemini_error_with_the_upstream_body() {
    let headers = IndexMap::new();
    let failure = decode_studio_agent_delete(&reply(404, &headers, "gone"), "req")
        .expect_err("a 404 is a failure");
    let StudioAgentError::Gemini { status, body, .. } = failure else {
        panic!("expected a Gemini error");
    };
    assert_eq!((status, body.as_str()), (404, "gone"));
}

#[test]
fn list_and_versions_map_the_native_page_fields() {
    let (agents, token) = page_of(
        AgentOperation::List,
        r#"{"agents":[{"id":"a","z":1}],"nextPageToken":"t"}"#,
    );
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["z"], Json::Number(Number::from(1_i64)));
    assert_eq!(token.as_deref(), Some("t"));
    let (versions, versions_token) = page_of(
        AgentOperation::Versions,
        r#"{"agentVersions":[{"name":"agents/a/versions/1"}]}"#,
    );
    assert_eq!(versions.len(), 1);
    assert_eq!(
        versions[0]["name"],
        Json::Text("agents/a/versions/1".to_owned())
    );
    assert_eq!(versions_token, None);
}

#[rstest]
#[case::not_json("not json")]
#[case::empty_object("{}")]
fn malformed_list_replies_are_empty_pages(#[case] body: &str) {
    let (agents, token) = page_of(AgentOperation::List, body);
    assert!(agents.is_empty());
    assert_eq!(token, None);
    let (versions, versions_token) = page_of(AgentOperation::Versions, body);
    assert!(versions.is_empty());
    assert_eq!(versions_token, None);
}

#[rstest]
#[case::array("[]", "root")]
#[case::null("null", "root")]
#[case::agents_null(r#"{"agents":null}"#, "agents")]
#[case::agents_text(r#"{"agents":"x"}"#, "agents")]
#[case::item_text(r#"{"agents":[1]}"#, "agents")]
#[case::token_number(r#"{"agents":[],"nextPageToken":5}"#, "nextPageToken")]
fn a_wrong_shaped_list_reply_fails(#[case] body: &str, #[case] field: &str) {
    let headers = IndexMap::new();
    let failure =
        decode_studio_agent_payload(&reply(200, &headers, body), AgentOperation::List, "req")
            .expect_err("a wrong-shaped page fails");
    assert!(matches!(failure, StudioAgentError::Shape { field: found } if found == field));
}

#[test]
fn page_root_extras_are_dropped_as_the_source_drops_them() {
    let headers = IndexMap::new();
    let decoded = decode_studio_agent_payload(
        &reply(200, &headers, r#"{"agents":[],"extra":1}"#),
        AgentOperation::List,
        "req",
    )
    .unwrap();
    let AgentPayload::Agents(page) = decoded.payload else {
        panic!("this reply is a page");
    };
    assert_eq!(
        dumped(&page),
        parse(r#"{"agents":[],"next_page_token":null}"#)
    );
}

#[rstest]
#[case::ok(200, "garbage")]
#[case::no_content(204, "")]
fn any_2xx_delete_synthesizes_the_requested_name(#[case] status: u16, #[case] body: &str) {
    let headers = IndexMap::new();
    let deleted = decode_studio_agent_delete(&reply(status, &headers, body), "my-agent").unwrap();
    assert_eq!(
        deleted,
        AgentDeleteResult {
            name: "my-agent".to_owned(),
            deleted: true,
        }
    );
    assert_eq!(
        dumped(&deleted),
        parse(r#"{"name":"my-agent","deleted":true}"#)
    );
}

#[test]
fn page_records_serialize_with_the_declared_field_names() {
    let page = AgentPage {
        agents: vec![IndexMap::from([(
            "id".to_owned(),
            Json::Text("a".to_owned()),
        )])],
        next_page_token: Some("t".to_owned()),
    };
    assert_eq!(
        dumped(&page),
        parse(r#"{"agents":[{"id":"a"}],"next_page_token":"t"}"#)
    );
    let versions = super::records::AgentVersionsPage {
        agent_versions: Vec::new(),
        next_page_token: None,
    };
    assert_eq!(
        dumped(&versions),
        parse(r#"{"agent_versions":[],"next_page_token":null}"#)
    );
}

#[test]
fn an_oversized_reply_is_refused() {
    let headers = IndexMap::new();
    let body = "x".repeat(RESPONSE_BODY_LIMIT.saturating_add(1));
    let failure =
        decode_studio_agent_payload(&reply(200, &headers, &body), AgentOperation::Get, "req")
            .expect_err("the reply is over the limit");
    assert!(matches!(
        failure,
        StudioAgentError::ResponseTooLarge { size, limit }
            if size == RESPONSE_BODY_LIMIT.saturating_add(1) && limit == RESPONSE_BODY_LIMIT
    ));
    let exact = "x".repeat(RESPONSE_BODY_LIMIT);
    decode_studio_agent_payload(&reply(200, &headers, &exact), AgentOperation::Get, "req")
        .expect("a body at the limit decodes");
}

#[rstest]
fn replays_the_studio_agent_payload_corpus(
    #[files("fuzz/corpus/studio_agent_payload/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a readable seed");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a utf-8 seed name");
    let headers = IndexMap::new();
    let response = RawResponse {
        status: 200,
        headers: &headers,
        body: &bytes,
    };
    let operations = [
        AgentOperation::Create,
        AgentOperation::Get,
        AgentOperation::List,
        AgentOperation::Versions,
    ];
    let outcomes: Vec<bool> = operations
        .into_iter()
        .map(|operation| decode_studio_agent_payload(&response, operation, "req").is_ok())
        .collect();
    if name.starts_with("invalid-json") {
        assert!(outcomes.iter().all(|ok| *ok), "{name} falls back");
    }
    if name.starts_with("wrong-shape") {
        assert!(outcomes.iter().all(|ok| !*ok), "{name} fails");
    }
    decode_studio_agent_delete(&response, "req").expect("a delete ignores the body");
    if name.starts_with("valid-create") {
        assert!(outcomes[0] && outcomes[1], "{name} is an agent record");
    }
    if name.starts_with("valid-list") {
        assert!(outcomes[2], "{name} is an agent page");
    }
    if name.starts_with("valid-versions") {
        assert!(outcomes[3], "{name} is a versions page");
    }
}
