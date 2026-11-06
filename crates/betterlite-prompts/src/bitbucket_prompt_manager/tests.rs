#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::BitBucketTemplateManager;
use super::error::ManagerError;
use super::parse::{YamlParser, parse_prompt_file};
use super::template::BitBucketPromptTemplate;
use super::yaml_basic::parse_yaml_basic;
use crate::arguments::json::Object;
use crate::bitbucket_client::BitBucketClient;
use crate::bitbucket_client::config::BitBucketConfig;

const CLOSED_BASE: &str = "http://127.0.0.1:1/2.0";

fn transport() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

fn config(base: &str, extra: &str) -> BitBucketConfig {
    let yaml =
        format!("workspace: ws\nrepository: repo\naccess_token: tok\n\"\": \"{base}\"\n{extra}\n");
    serde_saphyr::from_str(&yaml).unwrap()
}

fn object(yaml: &str) -> Object {
    serde_saphyr::from_str(yaml).unwrap()
}

fn manager_with(templates: &[(&str, &str)]) -> BitBucketTemplateManager {
    let mut prompts = IndexMap::new();
    for (id, content) in templates {
        let template = parse_prompt_file(content, id, YamlParser::Full).unwrap();
        prompts.insert((*id).to_owned(), template);
    }
    BitBucketTemplateManager {
        prompts,
        yaml_parser: YamlParser::Full,
        client: BitBucketClient::new(config(CLOSED_BASE, ""), transport()).unwrap(),
    }
}

async fn serve(listener: TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(head).unwrap()
}

async fn load(
    prompt_id: &str,
    extra: &str,
    status: &str,
    body: &str,
) -> (String, BitBucketTemplateManager, Result<(), ManagerError>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/2.0", listener.local_addr().unwrap());
    let mut manager = BitBucketTemplateManager::new(config(&base, extra), transport(), None)
        .await
        .unwrap();
    let (head, result) = tokio::join!(
        serve(listener, status, body),
        manager.load_prompt_from_bitbucket(prompt_id)
    );
    (head, manager, result)
}

#[test]
fn template_stores_the_metadata_fields_it_exposes() {
    let metadata = object(
        r#"{
        "model": "gpt-4", "temperature": 0.5, "max_tokens": 100,
        "input": {"schema": {"name": "string"}}, "top_p": 0.9,
    }"#,
    );
    let template = BitBucketPromptTemplate::new(
        "greet".to_owned(),
        "Hi {{name}}".to_owned(),
        metadata.clone(),
        None,
    )
    .unwrap();
    assert_eq!(template.template_id, "greet");
    assert_eq!(template.content, "Hi {{name}}");
    assert_eq!(template.model.as_deref(), Some("gpt-4"));
    assert_eq!(template.temperature.unwrap().to_string(), "0.5");
    assert_eq!(template.max_tokens.unwrap().to_string(), "100");
    assert_eq!(template.input_schema, object(r#"{"name": "string"}"#));
    assert_eq!(
        template.optional_params,
        object(r#"{"temperature": 0.5, "max_tokens": 100, "top_p": 0.9}"#)
    );
    assert_eq!(template.metadata, metadata);
}

#[test]
fn template_explicit_model_beats_the_metadata_model() {
    let metadata = object(r#"{"model": "gpt-4"}"#);
    let template = BitBucketPromptTemplate::new(
        "t".to_owned(),
        String::new(),
        metadata,
        Some("claude".to_owned()),
    )
    .unwrap();
    assert_eq!(template.model.as_deref(), Some("claude"));
}

#[test]
fn template_without_metadata_has_empty_defaults() {
    let template =
        BitBucketPromptTemplate::new("t".to_owned(), "body".to_owned(), Object::new(), None)
            .unwrap();
    assert_eq!(template.model, None);
    assert_eq!(template.temperature, None);
    assert_eq!(template.max_tokens, None);
    assert!(template.input_schema.is_empty());
    assert!(template.optional_params.is_empty());
}

#[rstest]
#[case::model_not_text(r#"{"model": 4}"#, "metadata field 'model' must be a string")]
#[case::temperature_not_number(
    r#"{"temperature": "hot"}"#,
    "metadata field 'temperature' must be a number"
)]
#[case::max_tokens_not_number(
    r#"{"max_tokens": [1]}"#,
    "metadata field 'max_tokens' must be a number"
)]
#[case::input_not_mapping(r#"{"input": ["a"]}"#, "metadata field 'input' must be a mapping")]
#[case::schema_not_mapping(
    r#"{"input": {"schema": "x"}}"#,
    "metadata field 'input.schema' must be a mapping"
)]
fn template_rejects_metadata_of_the_wrong_type(#[case] metadata: &str, #[case] expected: &str) {
    let error = BitBucketPromptTemplate::new("t".to_owned(), String::new(), object(metadata), None)
        .unwrap_err();
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::no_frontmatter("  Hello {{name}}  ", "{}", "  Hello {{name}}  ")]
#[case::frontmatter_is_split_off("---\nmodel: gpt-4\n---\nHello", r#"{"model": "gpt-4"}"#, "Hello")]
#[case::template_is_trimmed("---\nmodel: x\n---\n\n  Body  \n", r#"{"model": "x"}"#, "Body")]
#[case::later_delimiters_stay_in_the_body(
    "---\nmodel: x\n---\nA --- B",
    r#"{"model": "x"}"#,
    "A --- B"
)]
#[case::missing_closing_delimiter_keeps_the_file(
    "---\nmodel: x\nHello",
    "{}",
    "---\nmodel: x\nHello"
)]
#[case::delimiter_must_open_the_file(
    " ---\nmodel: x\n---\nHello",
    "{}",
    " ---\nmodel: x\n---\nHello"
)]
#[case::empty_frontmatter("---\n---\nbody", "{}", "body")]
#[case::invalid_yaml_erases_the_metadata("---\n: : [\n---\nbody", "{}", "body")]
#[case::unclosed_flow_erases_the_metadata("---\nmodel: [a\n---\nbody", "{}", "body")]
#[case::falsy_scalar_becomes_empty("---\n0\n---\nbody", "{}", "body")]
#[case::nested_yaml_is_kept(
    "---\ninput:\n  schema:\n    a: string\nstop: [1, 2]\n---\nbody",
    r#"{"input": {"schema": {"a": "string"}}, "stop": [1, 2]}"#,
    "body"
)]
fn parse_splits_frontmatter_and_keeps_bad_yaml_from_failing_the_load(
    #[case] content: &str,
    #[case] metadata: &str,
    #[case] body: &str,
) {
    let template = parse_prompt_file(content, "p", YamlParser::Full).unwrap();
    assert_eq!(template.metadata, object(metadata));
    assert_eq!(template.content, body);
}

#[test]
fn parse_keeps_the_last_value_of_a_repeated_key() {
    let template = parse_prompt_file("---\na: 1\na: 2\n---\nbody", "p", YamlParser::Full).unwrap();
    assert_eq!(template.metadata, object(r#"{"a": 2}"#));
}

#[rstest]
#[case::text("---\njust words\n---\nbody")]
#[case::list("---\n- a\n- b\n---\nbody")]
#[case::number("---\n5\n---\nbody")]
fn parse_rejects_frontmatter_that_is_not_a_mapping(#[case] content: &str) {
    let error = parse_prompt_file(content, "p", YamlParser::Full).unwrap_err();
    assert_eq!(error.to_string(), "the frontmatter must be a mapping");
}

#[test]
fn parse_reads_yaml_1_1_booleans_and_yaml_1_2_numbers() {
    let content = "---\nflag: yes\nletter: y\noctal: 012\nclock: 1:30\n---\nbody";
    let template = parse_prompt_file(content, "p", YamlParser::Full).unwrap();
    assert_eq!(
        template.metadata,
        object(r#"{"flag": true, "letter": true, "octal": 12.0, "clock": "1:30"}"#)
    );
}

#[test]
fn parse_basic_parser_reads_simple_key_value_frontmatter() {
    let content = "---\nmodel: gpt-4\ntemperature: 0.5\nstream: true\n---\nhi";
    let template = parse_prompt_file(content, "p", YamlParser::Basic).unwrap();
    assert_eq!(
        template.metadata,
        object(r#"{"model": "gpt-4", "temperature": 0.5, "stream": true}"#)
    );
    assert_eq!(template.model.as_deref(), Some("gpt-4"));
}

#[test]
fn parse_basic_parser_error_is_not_swallowed() {
    let error = parse_prompt_file("---\nv: 1.2.3\n---\nhi", "p", YamlParser::Basic).unwrap_err();
    assert_eq!(error.to_string(), "'1.2.3' is not a number");
}

#[rstest]
#[case::true_value("a: true", r#"{"a": true}"#)]
#[case::mixed_case_false("a: False", r#"{"a": false}"#)]
#[case::integer("a: 5", r#"{"a": 5}"#)]
#[case::leading_zeros("a: 007", r#"{"a": 7}"#)]
#[case::float("a: 1.5", r#"{"a": 1.5}"#)]
#[case::trailing_dot("a: 1.", r#"{"a": 1.0}"#)]
#[case::leading_dot("a: .5", r#"{"a": 0.5}"#)]
#[case::double_quoted("a: \"x y\"", r#"{"a": "x y"}"#)]
#[case::single_quoted("a: 'x'", r#"{"a": "x"}"#)]
#[case::empty_value("a:", r#"{"a": ""}"#)]
#[case::negative_stays_text("a: -5", r#"{"a": "-5"}"#)]
#[case::only_dots_stay_text("a: ...", r#"{"a": "..."}"#)]
#[case::value_may_hold_colons("url: http://x:80/p", r#"{"url": "http://x:80/p"}"#)]
#[case::key_and_value_are_stripped("  key  :   v  ", r#"{"key": "v"}"#)]
#[case::comment_lines_are_skipped("# a: 1\nb: 2", r#"{"b": 2}"#)]
#[case::lines_without_a_colon_are_skipped("plain\nb: 2", r#"{"b": 2}"#)]
#[case::crlf_lines("a: 1\r\nb: x\r\n", r#"{"a": 1, "b": "x"}"#)]
#[case::last_duplicate_wins("a: 1\na: 2", r#"{"a": 2}"#)]
fn yaml_basic_reads_typed_key_value_lines(#[case] yaml: &str, #[case] expected: &str) {
    assert_eq!(parse_yaml_basic(yaml).unwrap(), object(expected));
}

#[rstest]
#[case::two_dots("a: 1.2.3", "'1.2.3' is not a number")]
#[case::dot_run("a: 1..2", "'1..2' is not a number")]
fn yaml_basic_rejects_a_malformed_number(#[case] yaml: &str, #[case] expected: &str) {
    assert_eq!(parse_yaml_basic(yaml).unwrap_err().to_string(), expected);
}

#[rstest]
#[case::substitutes_variables("Hello {{ name }}!", r#"{"name": "Ann"}"#, "Hello Ann!")]
#[case::loops_and_filters(
    "{% for x in xs %}{{ x|upper }},{% endfor %}",
    r#"{"xs": ["a", "b"]}"#,
    "A,B,"
)]
#[case::string_templates_autoescape_html("{{ s }}", r#"{"s": "<b>"}"#, "&lt;b&gt;")]
#[case::missing_variable_is_empty("[{{ gone }}]", "{}", "[]")]
fn render_fills_the_template_with_the_variables(
    #[case] content: &str,
    #[case] variables: &str,
    #[case] expected: &str,
) {
    let manager = manager_with(&[("t", content)]);
    assert_eq!(
        manager.render_template("t", &object(variables)).unwrap(),
        expected
    );
}

#[rstest]
#[case::dunder_class("{{ ''.__class__ }}")]
#[case::dunder_mro("{{ ''.__class__.__mro__ }}")]
#[case::subclasses("{{ ().__class__.__base__.__subclasses__() }}")]
// the sandbox has no Python objects to reach, so a dunder lookup renders nothing or fails
fn render_sandbox_exposes_no_interpreter_internals(#[case] content: &str) {
    let manager = manager_with(&[("t", content)]);
    let rendered = manager
        .render_template("t", &Object::new())
        .unwrap_or_default();
    assert_eq!(rendered, "");
}

#[test]
fn render_stops_a_template_over_the_byte_budget() {
    let content = "x".repeat(0x0001_0001);
    let manager = manager_with(&[("big", content.as_str())]);
    let error = manager.render_template("big", &Object::new()).unwrap_err();
    assert_eq!(error.to_string(), "prompt template exceeds its byte budget");
}

#[test]
fn render_stops_a_template_that_never_ends() {
    let manager = manager_with(&[(
        "loop",
        "{% for x in range(100000) %}{% for y in range(100000) %}{% endfor %}{% endfor %}",
    )]);
    let error = manager.render_template("loop", &Object::new()).unwrap_err();
    assert!(error.to_string().contains("fuel"));
}

#[test]
fn render_unknown_template_is_not_found() {
    let manager = manager_with(&[]);
    let error = manager.render_template("nope", &Object::new()).unwrap_err();
    assert_eq!(error.to_string(), "Template 'nope' not found");
}

#[test]
fn render_does_not_change_the_cached_template() {
    let manager = manager_with(&[("t", "Hi {{ name }}")]);
    manager
        .render_template("t", &object(r#"{"name": "Ann"}"#))
        .unwrap();
    assert_eq!(manager.prompts.get("t").unwrap().content, "Hi {{ name }}");
}

#[test]
fn list_of_cached_template_ids_keeps_load_order() {
    let manager = manager_with(&[("zeta", "z"), ("alpha", "a"), ("mid", "m")]);
    assert_eq!(
        manager.prompts.keys().collect::<Vec<_>>(),
        ["zeta", "alpha", "mid"]
    );
}

#[test]
fn list_of_cached_template_ids_is_empty_before_anything_loads() {
    assert!(manager_with(&[]).prompts.is_empty());
}

#[tokio::test]
async fn load_stores_the_parsed_template_under_the_prompt_id() {
    let body = "---\nmodel: gpt-4\n---\nHi {{ name }}";
    let (head, manager, result) = load("greeting", "", "200 OK", body).await;
    result.unwrap();
    assert!(head.starts_with("GET /2.0/repositories/ws/repo/src/main/greeting.prompt HTTP/1.1"));
    let template = manager.prompts.get("greeting").unwrap();
    assert_eq!(template.content, "Hi {{ name }}");
    assert_eq!(template.model.as_deref(), Some("gpt-4"));
    assert_eq!(manager.prompts.keys().collect::<Vec<_>>(), ["greeting"]);
}

#[tokio::test]
async fn load_get_template_returns_the_stored_template_and_none_for_another_id() {
    let (_, manager, result) = load("greeting", "", "200 OK", "Hi").await;
    result.unwrap();
    assert_eq!(
        manager
            .prompts
            .get("greeting")
            .map(|template| template.content.as_str()),
        Some("Hi")
    );
    assert!(manager.prompts.get("other").is_none());
}

#[rstest]
#[case::missing_file("404 Not Found", "ignored")]
#[case::empty_file("200 OK", "")]
#[tokio::test]
async fn load_keeps_nothing_for_a_missing_or_empty_file(#[case] status: &str, #[case] body: &str) {
    let (_, manager, result) = load("greeting", "", status, body).await;
    result.unwrap();
    assert!(manager.prompts.get("greeting").is_none());
    assert!(manager.prompts.is_empty());
}

#[tokio::test]
async fn load_fetch_failure_names_the_prompt() {
    let (_, manager, result) = load("greeting", "", "500 Internal Server Error", "x").await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Failed to load prompt 'greeting' from BitBucket: Failed to fetch file 'greeting.prompt': BitBucket answered with status 500"
    );
    assert!(manager.prompts.get("greeting").is_none());
}

#[tokio::test]
async fn load_parse_failure_names_the_prompt() {
    let (_, manager, result) = load("bad", "", "200 OK", "---\nnot a mapping\n---\nx").await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Failed to load prompt 'bad' from BitBucket: the frontmatter must be a mapping"
    );
    assert!(manager.prompts.get("bad").is_none());
}

#[tokio::test]
async fn load_during_construction_fetches_the_named_prompt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/2.0", listener.local_addr().unwrap());
    let (_, manager) = tokio::join!(
        serve(listener, "200 OK", "Hello"),
        BitBucketTemplateManager::new(config(&base, ""), transport(), Some("welcome"))
    );
    assert_eq!(
        manager.unwrap().prompts.keys().collect::<Vec<_>>(),
        ["welcome"]
    );
}

#[tokio::test]
async fn load_construction_fails_when_the_named_prompt_cannot_be_fetched() {
    let error =
        BitBucketTemplateManager::new(config(CLOSED_BASE, ""), transport(), Some("welcome"))
            .await
            .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load prompt 'welcome' from BitBucket: Error fetching file 'welcome.prompt'"
    );
}

#[tokio::test]
async fn path_prompts_path_key_is_not_applied_because_the_pin_has_none() {
    let (head, _, result) = load("greeting", "prompts_path: sub/dir", "200 OK", "Hi").await;
    result.unwrap();
    assert!(head.starts_with("GET /2.0/repositories/ws/repo/src/main/greeting.prompt HTTP/1.1"));
}

#[tokio::test]
async fn path_prompt_id_is_quoted_and_traversal_is_refused() {
    let (head, _, result) = load("team a/greeting", "", "200 OK", "Hi").await;
    result.unwrap();
    assert!(head.contains("/src/main/team%20a/greeting.prompt HTTP/1.1"));

    let mut manager = BitBucketTemplateManager::new(config(CLOSED_BASE, ""), transport(), None)
        .await
        .unwrap();
    let error = manager
        .load_prompt_from_bitbucket("../greeting")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load prompt '../greeting' from BitBucket: Invalid file path \"../greeting.prompt\": path traversal detected"
    );
}

#[rstest]
fn replays_the_bitbucket_prompt_file_corpus(
    #[files("fuzz/corpus/bitbucket_prompt_file/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(content) = str::from_utf8(&bytes) else {
        return;
    };
    drop(parse_prompt_file(content, "fuzz", YamlParser::Full));
    drop(parse_prompt_file(content, "fuzz", YamlParser::Basic));
    drop(parse_yaml_basic(content));
}
