#![cfg(test)]

use super::{auto_escape_for_name, build};
use crate::arguments::json::{Json, Object};
use minijinja::AutoEscape;
use minijinja::value::Serde;
use rstest::rstest;

#[rstest]
fn uses_handlebars_style_delimiters_matching_the_dotprompt_spec() {
    let env = build();
    let syntax = env.syntax();
    assert_eq!(syntax.variable_delimiters(), ("{{", "}}"));
    assert_eq!(syntax.block_delimiters(), ("{%", "%}"));
    assert_eq!(syntax.comment_delimiters(), ("{#", "#}"));
}

#[rstest]
#[case::html("page.html", AutoEscape::Html)]
#[case::xml("feed.xml", AutoEscape::Html)]
#[case::string_template("<string>", AutoEscape::None)]
#[case::text("notes.txt", AutoEscape::None)]
fn selects_autoescape_for_html_and_xml_only(#[case] name: &str, #[case] expected: AutoEscape) {
    assert_eq!(auto_escape_for_name(name), expected);
}

#[rstest]
fn rejects_calling_an_unknown_mutating_method_on_a_caller_supplied_list() {
    let env = build();
    let items = vec![
        Json::Number(serde_json::Number::from(1_i64)),
        Json::Number(serde_json::Number::from(2_i64)),
        Json::Number(serde_json::Number::from(3_i64)),
    ];
    let mut variables = Object::new();
    variables.insert("items".to_owned(), Json::List(items));
    let context = Serde(variables);
    let result = env.render_str("{{ items.append(4) }}", context);
    result.unwrap_err();
}

#[rstest]
fn a_runaway_loop_is_stopped_by_the_fuel_budget() {
    let env = build();
    let result = env.render_str("{% for i in range(100000000) %}{{ i }}{% endfor %}", ());
    result.unwrap_err();
}
