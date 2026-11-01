#![cfg(test)]

use super::{auto_escape_for_name, build};
use crate::arguments::json::{Json, Object};
use minijinja::AutoEscape;
use minijinja::value::Serde;
use rstest::rstest;
use std::borrow::Cow;

fn markupsafe() -> AutoEscape {
    AutoEscape::Custom(Cow::Borrowed("markupsafe"))
}

#[rstest]
fn uses_handlebars_style_delimiters_matching_the_dotprompt_spec() {
    let env = build();
    let syntax = env.syntax();
    assert_eq!(syntax.variable_delimiters(), ("{{", "}}"));
    assert_eq!(syntax.block_delimiters(), ("{%", "%}"));
    assert_eq!(syntax.comment_delimiters(), ("{#", "#}"));
}

#[rstest]
#[case::html("page.html", markupsafe())]
#[case::xml("feed.xml", markupsafe())]
#[case::string_template("<string>", markupsafe())]
#[case::text("notes.txt", AutoEscape::None)]
fn selects_autoescape_for_html_and_xml_only(#[case] name: &str, #[case] expected: AutoEscape) {
    assert_eq!(auto_escape_for_name(name), expected);
}

#[rstest]
fn escapes_html_special_characters_with_markupsafes_exact_table() {
    let env = build();
    let mut variables = Object::new();
    variables.insert(
        "name".to_owned(),
        Json::Text("O'Brien & <Sons> \"Ltd\"/Co".to_owned()),
    );
    let rendered = env.render_str("{{ name }}", Serde(variables)).unwrap();
    assert_eq!(rendered, "O&#39;Brien &amp; &lt;Sons&gt; &#34;Ltd&#34;/Co");
}

#[rstest]
fn calling_a_known_non_mutating_method_renders() {
    let env = build();
    let mut variables = Object::new();
    variables.insert("name".to_owned(), Json::Text("ada".to_owned()));
    let upper = env
        .render_str("{{ name.upper() }}", Serde(variables))
        .unwrap();
    assert_eq!(upper, "ADA");

    let mut dict = Object::new();
    dict.insert("a".to_owned(), Json::Text("one".to_owned()));
    let mut pairs = Object::new();
    pairs.insert("d".to_owned(), Json::Object(dict));
    let items = env
        .render_str(
            "{% for k, v in d.items() %}{{ k }}={{ v }}{% endfor %}",
            Serde(pairs),
        )
        .unwrap();
    assert_eq!(items, "a=one");
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
    let template = "{% for i in range(100000) %}{% for j in range(100000) %}{{ i }}{{ j }}{% endfor %}{% endfor %}";
    let error = env.render_str(template, ()).unwrap_err();
    assert!(error.to_string().contains("fuel"));
}
