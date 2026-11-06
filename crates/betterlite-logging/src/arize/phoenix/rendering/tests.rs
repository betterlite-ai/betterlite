#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptParams;
use rstest::rstest;

use super::{RenderError, render_messages, sandbox};
use crate::arize::phoenix::parsing::parse_prompt_data;
use crate::arize::phoenix::records::PromptTemplate;

fn template(parts: &str) -> PromptTemplate {
    let data = format!(r#"{{"template":{{"messages":[{{"role":"user","content":[{parts}]}}]}}}}"#);
    parse_prompt_data(serde_json::from_str(&data).unwrap(), "v1").unwrap()
}

fn variables(text: &str) -> PromptParams {
    serde_json::from_str(text).unwrap()
}

fn text_part(source: &str) -> String {
    let fields = BTreeMap::from([("type", "text"), ("text", source)]);
    serde_json::to_string(&fields).unwrap()
}

fn render(source: &str, values: &str) -> String {
    let part = text_part(source);
    let messages = render_messages(&sandbox(), &template(&part), Some(&variables(values))).unwrap();
    let Json::Object(message) = &messages[0] else {
        panic!("message is not an object")
    };
    let Json::Text(content) = &message["content"] else {
        panic!("content is not text")
    };
    content.clone()
}

#[rstest]
#[case::variable("Hello {{ name }}!", r#"{"name":"Ada"}"#, "Hello Ada!")]
#[case::missing_variable_renders_empty("[{{ nobody }}]", "{}", "[]")]
#[case::block_delimiters("{% if on %}yes{% else %}no{% endif %}", r#"{"on":true}"#, "yes")]
#[case::loop_over_a_list(
    "{% for n in items %}{{ n }},{% endfor %}",
    r#"{"items":[1,2,3]}"#,
    "1,2,3,"
)]
#[case::comment_delimiters("a{# hidden #}b", "{}", "ab")]
#[case::filters("{{ name | upper }}", r#"{"name":"ada"}"#, "ADA")]
#[case::integer_arithmetic("{{ count + 1 }}", r#"{"count":3}"#, "4")]
#[case::nested_lookup("{{ user.name }}", r#"{"user":{"name":"Ada"}}"#, "Ada")]
#[case::single_braces_stay_literal("{name}", r#"{"name":"Ada"}"#, "{name}")]
fn rendering_substitutes_variables_and_runs_jinja_syntax(
    #[case] source: &str,
    #[case] values: &str,
    #[case] expected: &str,
) {
    assert_eq!(render(source, values), expected);
}

#[rstest]
#[case::angle_brackets("<b>", "&lt;b&gt;")]
#[case::ampersand("a & b", "a &amp; b")]
#[case::quotes(r#"it's "x""#, "it&#39;s &#34;x&#34;")]
#[case::slash_is_not_escaped("a/b", "a/b")]
fn rendering_autoescapes_variables_with_markupsafe_entities(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let values = serde_json::to_string(&BTreeMap::from([("v", value)])).unwrap();
    assert_eq!(render("{{ v }}", &values), expected);
}

#[test]
fn rendering_does_not_escape_the_template_text_itself() {
    assert_eq!(render("<p>{{ v }}</p>", r#"{"v":"<"}"#), "<p>&lt;</p>");
}

#[test]
fn rendering_does_not_escape_values_marked_safe() {
    assert_eq!(render("{{ v | safe }}", r#"{"v":"<b>"}"#), "<b>");
}

#[rstest]
#[case::class_chain("{{ ''.__class__.__mro__ }}")]
#[case::globals_chain("{{ name.__init__.__globals__ }}")]
fn rendering_sandbox_does_not_reach_host_attributes(#[case] source: &str) {
    let part = text_part(source);
    let rendered = render_messages(
        &sandbox(),
        &template(&part),
        Some(&variables(r#"{"name":"x"}"#)),
    );
    match rendered {
        Ok(messages) => {
            let Json::Object(message) = &messages[0] else {
                panic!("message is not an object")
            };
            assert_eq!(message["content"], Json::Text(String::new()));
        }
        Err(RenderError::Template { .. }) => {}
        Err(other) => panic!("unexpected error {other}"),
    }
}

#[test]
fn rendering_stops_a_template_that_loops_past_its_fuel() {
    let part =
        text_part("{% for i in range(100000) %}{% for j in range(100) %}x{% endfor %}{% endfor %}");
    let error = render_messages(&sandbox(), &template(&part), None).unwrap_err();
    let RenderError::Template { source } = error else {
        panic!("not a template error: {error}")
    };
    assert_eq!(source.kind(), minijinja::ErrorKind::OutOfFuel);
}

#[test]
fn rendering_reports_a_template_syntax_error() {
    let part = r#"{"type":"text","text":"{% if %}"}"#;
    let error = render_messages(&sandbox(), &template(part), None).unwrap_err();
    assert!(matches!(error, RenderError::Template { .. }));
}

#[test]
fn rendering_joins_text_parts_of_one_message_with_a_space() {
    let parts = r#"{"type":"text","text":"a {{ x }}"},{"type":"text","text":"b"}"#;
    let messages =
        render_messages(&sandbox(), &template(parts), Some(&variables(r#"{"x":1}"#))).unwrap();
    assert_eq!(
        messages[0],
        serde_json::from_str::<Json>(r#"{"role":"user","content":"a 1 b"}"#).unwrap()
    );
}

#[test]
fn rendering_keeps_each_role_and_renders_every_message() {
    let data = r#"{"template":{"messages":[{"role":"system","content":[{"type":"text","text":"S {{ v }}"}]},{"content":[{"type":"text","text":"U {{ v }}"}]}]}}"#;
    let template = parse_prompt_data(serde_json::from_str(data).unwrap(), "v1").unwrap();
    let messages =
        render_messages(&sandbox(), &template, Some(&variables(r#"{"v":"x"}"#))).unwrap();
    let expected: Vec<Json> = serde_json::from_str(
        r#"[{"role":"system","content":"S x"},{"role":"user","content":"U x"}]"#,
    )
    .unwrap();
    assert_eq!(messages, expected);
}

#[test]
fn rendering_a_message_without_content_gives_an_empty_string() {
    let data = r#"{"template":{"messages":[{"role":"user"}]}}"#;
    let template = parse_prompt_data(serde_json::from_str(data).unwrap(), "v1").unwrap();
    let messages = render_messages(&sandbox(), &template, None).unwrap();
    assert_eq!(
        messages[0],
        serde_json::from_str::<Json>(r#"{"role":"user","content":""}"#).unwrap()
    );
}

#[test]
fn rendering_rejects_a_non_text_part_like_the_python_join_does() {
    let parts = r#"{"type":"text","text":"a"},{"type":"image","text":"b"}"#;
    let error = render_messages(&sandbox(), &template(parts), None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "sequence item 1: expected str instance, dict found"
    );
}
