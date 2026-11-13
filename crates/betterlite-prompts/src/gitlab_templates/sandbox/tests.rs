#![cfg(test)]

use rstest::rstest;

use super::render_sandboxed;
use crate::arguments::json::{Json, Object};

fn variables(json_text: &str) -> Object {
    serde_json::from_str(json_text).unwrap()
}

#[rstest]
#[case::variable("a{{ x }}b", "a1b")]
#[case::block("{% if x == 1 %}one{% else %}other{% endif %}", "one")]
#[case::comment("a{# hidden #}b", "ab")]
#[case::loop_block("{% for i in range(3) %}{{ i }}{% endfor %}", "012")]
fn jinja_delimiters_are_double_braces_percent_blocks_and_hash_comments(
    #[case] content: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        render_sandboxed("t", content, &variables(r#"{"x": 1}"#)).unwrap(),
        expected
    );
}

#[test]
fn other_delimiters_render_as_plain_text() {
    let scope = variables(r#"{"x": 1}"#);
    assert_eq!(
        render_sandboxed("t", "{x} [% x %] <# x #>", &scope).unwrap(),
        "{x} [% x %] <# x #>"
    );
}

#[test]
fn handlebars_blocks_are_a_syntax_error() {
    let scope = variables(r#"{"x": 1}"#);
    render_sandboxed("t", "{{#if x}}y{{/if}}", &scope).unwrap_err();
}

#[rstest]
#[case::tags("<b>", "&lt;b&gt;")]
#[case::ampersand_and_quotes("a & \"b\" 'c'", "a &amp; &#34;b&#34; &#39;c&#39;")]
#[case::slash_is_kept("a/b", "a/b")]
fn string_templates_html_escape_variables_like_select_autoescape(
    #[case] value: &str,
    #[case] expected: &str,
) {
    let scope = Object::from([("v".to_owned(), Json::Text(value.to_owned()))]);
    assert_eq!(render_sandboxed("t", "{{ v }}", &scope).unwrap(), expected);
}

#[test]
fn the_safe_filter_opts_out_of_escaping() {
    let scope = variables(r#"{"v": "<b>"}"#);
    assert_eq!(
        render_sandboxed("t", "{{ v|safe }}", &scope).unwrap(),
        "<b>"
    );
}

#[rstest]
#[case::mutating_a_list("{{ items.append(4) }}")]
#[case::mutating_a_dict("{{ d.update({'a': 1}) }}")]
#[case::popping("{{ items.pop() }}")]
fn mutating_methods_on_caller_data_are_rejected(#[case] content: &str) {
    let scope = variables(r#"{"items": [1, 2, 3], "d": {"k": 1}}"#);
    render_sandboxed("t", content, &scope).unwrap_err();
}

#[rstest]
#[case::dunder_class("{{ name.__class__ }}")]
#[case::init_globals("{{ name.__class__.__init__.__globals__ }}")]
#[case::dunder_mro("{{ ''.__class__.__mro__ }}")]
#[case::subclasses("{{ ''.__class__.__base__.__subclasses__() }}")]
fn attribute_traversal_does_not_reach_python_internals(#[case] content: &str) {
    let rendered = render_sandboxed("t", content, &variables(r#"{"name": "ada"}"#));
    assert!(rendered.is_err() || rendered.is_ok_and(|text| text.is_empty()));
}

#[test]
fn templates_cannot_rebind_the_caller_variables_for_later_renders() {
    let scope = variables(r#"{"x": 1}"#);
    assert_eq!(
        render_sandboxed("t", "{% set x = 2 %}{{ x }}", &scope).unwrap(),
        "2"
    );
    assert_eq!(render_sandboxed("t", "{{ x }}", &scope).unwrap(), "1");
}

#[test]
fn a_runaway_loop_is_stopped_before_it_hangs_the_gateway() {
    let template =
        "{% for i in range(100000) %}{% for j in range(100000) %}{{ i }}{% endfor %}{% endfor %}";
    let error = render_sandboxed("t", template, &Object::new()).unwrap_err();
    assert!(error.to_string().contains("fuel"));
}
