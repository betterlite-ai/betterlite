#![cfg(test)]

use rstest::rstest;

use super::support::{object, routes};
use crate::api::file_batch_guardrails::call_type::{call_type_from_url, scannable_call_type};
use crate::api::file_batch_guardrails::record_json::RecordJson;
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::url_path::url_path;
use crate::routes::CallType;

#[rstest]
#[case::chat("/chat/completions", CallType::Acompletion)]
#[case::chat_v1("/v1/chat/completions", CallType::Acompletion)]
#[case::chat_engine("/engines/gpt-4o/chat/completions", CallType::Acompletion)]
#[case::chat_deployment("/openai/deployments/gpt-4o/chat/completions", CallType::Acompletion)]
#[case::text("/completions", CallType::AtextCompletion)]
#[case::text_v1("/v1/completions", CallType::AtextCompletion)]
#[case::text_engine("/engines/gpt-4o/completions", CallType::AtextCompletion)]
#[case::text_deployment("/openai/deployments/gpt-4o/completions", CallType::AtextCompletion)]
#[case::embedding("/embeddings", CallType::Aembedding)]
#[case::embedding_v1("/v1/embeddings", CallType::Aembedding)]
#[case::embedding_engine("/engines/text-embedding-3-small/embeddings", CallType::Aembedding)]
#[case::embedding_deployment(
    "/openai/deployments/text-embedding-3-small/embeddings",
    CallType::Aembedding
)]
#[case::responses("/responses", CallType::Aresponses)]
#[case::responses_v1("/v1/responses", CallType::Aresponses)]
#[case::responses_openai("/openai/v1/responses", CallType::Aresponses)]
#[case::response_id("/responses/r_1", CallType::Aresponses)]
#[case::response_id_v1("/v1/responses/r_1", CallType::Aresponses)]
#[case::response_id_openai("/openai/v1/responses/r_1", CallType::Aresponses)]
#[case::anthropic("/anthropic/v1/messages", CallType::AnthropicMessages)]
#[case::messages("/v1/messages", CallType::AnthropicMessages)]
#[case::absolute("https://example.invalid/v1/responses/?x=1", CallType::Aresponses)]
#[case::trailing_slashes("/v1/embeddings//", CallType::Aembedding)]
#[case::fragment("/v1/completions#part", CallType::AtextCompletion)]
#[case::leading_controls(" \u{1}/v1/messages", CallType::AnthropicMessages)]
#[case::embedded_tab("/v1/chat/\tcompletions", CallType::Acompletion)]
#[case::bracketed_host("http://[::1]:4000/v1/embeddings", CallType::Aembedding)]
#[case::scoped_host("http://[fe80::1%25eth0]/v1/embeddings", CallType::Aembedding)]
#[case::future_host("http://[v1.x]/v1/embeddings", CallType::Aembedding)]
#[case::user_info("http://user@[::1]/v1/embeddings", CallType::Aembedding)]
fn a_supported_route_names_its_call_type(#[case] url: &str, #[case] expected: CallType) {
    assert_eq!(call_type_from_url(&routes(), url), Some(expected));
}

#[rstest]
#[case::input_items("/responses/r_1/input_items")]
#[case::images("/v1/images/generations")]
#[case::unknown("/v1/unknown")]
#[case::empty("")]
#[case::scheme_eats_host("localhost:4000/v1/chat/completions")]
#[case::unclosed_bracket("https://[::1/v1/chat/completions")]
#[case::stray_closer("https://::1]/v1/chat/completions")]
#[case::text_before_bracket("https://a[::1]/v1/chat/completions")]
#[case::text_after_bracket("https://[::1]x/v1/chat/completions")]
#[case::ipv4_in_brackets("https://[127.0.0.1]/v1/chat/completions")]
#[case::not_an_address("https://[host]/v1/chat/completions")]
#[case::empty_scope("https://[fe80::1%]/v1/chat/completions")]
#[case::double_scope("https://[fe80::1%a%b]/v1/chat/completions")]
#[case::future_without_version("https://[v.x]/v1/chat/completions")]
#[case::future_bad_version("https://[vg.x]/v1/chat/completions")]
#[case::future_without_address("https://[v1.]/v1/chat/completions")]
#[case::future_without_point("https://[v1]/v1/chat/completions")]
#[case::bracket_only_in_user("https://[a]@host/v1/chat/completions")]
fn an_unscannable_or_unreadable_url_resolves_nothing(#[case] url: &str) {
    assert_eq!(call_type_from_url(&routes(), url), None);
}

#[rstest]
#[case::relative("/v1/chat/completions", Some("/v1/chat/completions"))]
#[case::scheme_and_query("HTTPS://h:1/a/b?c=d#e", Some("/a/b"))]
#[case::query_before_fragment("/a?b#c", Some("/a"))]
#[case::network_path("//host/a", Some("/a"))]
#[case::authority_only("https://host", Some(""))]
#[case::authority_then_query("https://host?x=/a", Some(""))]
#[case::digit_first_scheme("1a:/b", Some("1a:/b"))]
#[case::odd_scheme_character("a_b:/c", Some("a_b:/c"))]
#[case::scheme_characters("a+b-c.d:/e", Some("/e"))]
#[case::removes_newlines("/a\r\n/b", Some("/a/b"))]
#[case::keeps_trailing_space("/a ", Some("/a "))]
#[case::port_after_bracket("http://[::1]:80/a", Some("/a"))]
#[case::user_before_bracket("http://u:p@[::1]/a", Some("/a"))]
#[case::malformed("http://[::1/a", None)]
fn reduces_a_url_to_its_path(#[case] url: &str, #[case] expected: Option<&str>) {
    assert_eq!(url_path(url).as_deref(), expected);
}

fn scanned_as(url: Option<&RecordJson>, body: &str) -> Option<CallType> {
    scannable_call_type(&routes(), url, &object(body))
}

fn text(url: &str) -> RecordJson {
    RecordJson::Text(RecordText::from(url))
}

#[rstest]
#[case::all_null(
    None,
    r#"{"messages":null,"prompt":null,"input":null}"#,
    Some(CallType::Acompletion)
)]
#[case::prompt_then_input(
    None,
    r#"{"prompt":null,"input":null}"#,
    Some(CallType::AtextCompletion)
)]
#[case::input_only(None, r#"{"input":null}"#, Some(CallType::Aembedding))]
#[case::order_is_the_table_order(None, r#"{"input":1,"messages":[]}"#, Some(CallType::Acompletion))]
#[case::nothing(None, "{}", None)]
#[case::unknown_url(
    Some(&text("/v1/unknown")),
    r#"{"prompt":"x"}"#,
    Some(CallType::AtextCompletion)
)]
#[case::empty_url(Some(&text("")), r#"{"input":"x"}"#, Some(CallType::Aembedding))]
#[case::number_url(Some(&RecordJson::Number("7".to_owned())), r#"{"input":"x"}"#, Some(CallType::Aembedding))]
#[case::null_url(
    Some(&RecordJson::Null),
    r#"{"messages":[]}"#,
    Some(CallType::Acompletion)
)]
#[case::list_url(Some(&RecordJson::List(Vec::new())), "{}", None)]
#[case::malformed_url(
    Some(&text("https://[::1/v1/responses")),
    r#"{"input":"x"}"#,
    Some(CallType::Aembedding)
)]
#[case::images_falls_back(
    Some(&text("/v1/images/generations")),
    r#"{"messages":[]}"#,
    Some(CallType::Acompletion)
)]
#[case::input_items_falls_back(Some(&text("/responses/r_1/input_items")), "{}", None)]
#[case::url_wins(
    Some(&text("/v1/embeddings")),
    r#"{"messages":[]}"#,
    Some(CallType::Aembedding)
)]
#[case::responses_input(Some(&text("/v1/responses")), r#"{"input":"x"}"#, Some(CallType::Aresponses))]
fn falls_back_to_the_body_shape(
    #[case] url: Option<&RecordJson>,
    #[case] body: &str,
    #[case] expected: Option<CallType>,
) {
    assert_eq!(scanned_as(url, body), expected);
}

#[test]
fn a_surrogate_in_a_url_still_matches_a_placeholder() {
    let mut code_points: Vec<u32> = "/engines/".chars().map(u32::from).collect();
    code_points.push(0xD800);
    code_points.extend("/embeddings".chars().map(u32::from));
    let url = Some(RecordJson::Text(RecordText::from_code_points(&code_points)));
    assert_eq!(scanned_as(url.as_ref(), "{}"), Some(CallType::Aembedding));
}

#[test]
fn a_url_past_the_route_limit_falls_back() {
    let long = format!("/engines/{}/embeddings", "m".repeat(5000));
    assert_eq!(call_type_from_url(&routes(), &long), None);
    assert_eq!(
        scanned_as(Some(&text(&long)), r#"{"prompt":"x"}"#),
        Some(CallType::AtextCompletion)
    );
}
