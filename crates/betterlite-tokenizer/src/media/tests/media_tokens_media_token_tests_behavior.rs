use rstest::rstest;

use crate::media::count::Warning;
use crate::media::settings::ImageCounting;

use super::support::{self, CaseOptions};

const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABA";
const TRUNCATED: &str = "data:image/png;base64,iVBORw0KGgo=";
const REMOTE: &str = "https://example.com/image.png";

const EXACT: CaseOptions = CaseOptions {
    counting: ImageCounting::Exact,
    fallback: None,
    image_default_count: 250,
};
const DEFAULT: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: None,
    image_default_count: 250,
};
const DEFAULT_123: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: None,
    image_default_count: 123,
};
const FALLBACK: CaseOptions = CaseOptions {
    counting: ImageCounting::Exact,
    fallback: Some(777),
    image_default_count: 250,
};
const MAX_COUNT: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: None,
    image_default_count: u64::MAX,
};
const MAX_COUNT_FALLBACK: CaseOptions = CaseOptions {
    counting: ImageCounting::DefaultCount,
    fallback: Some(7),
    image_default_count: u64::MAX,
};

#[derive(Debug, Clone, Copy)]
enum Expected {
    Counted {
        tokens: u64,
        remote_warnings: usize,
        fallback_warnings: usize,
    },
    Failed(&'static str),
    Invalid,
}

const fn tokens(tokens: u64) -> Expected {
    Expected::Counted {
        tokens,
        remote_warnings: 0,
        fallback_warnings: 0,
    }
}

const fn remote_tokens(tokens: u64) -> Expected {
    Expected::Counted {
        tokens,
        remote_warnings: 1,
        fallback_warnings: 0,
    }
}

const fn fallback_tokens(tokens: u64) -> Expected {
    Expected::Counted {
        tokens,
        remote_warnings: 0,
        fallback_warnings: 1,
    }
}

const fn failed(message: &'static str) -> Expected {
    Expected::Failed(message)
}

fn plain(inner: &str) -> String {
    format!(r#"{{"content":[{inner}]}}"#)
}

fn image_url(url: &str, detail: &str) -> String {
    format!(
        r#"{{"content":[{{"type":"image_url","image_url":{{"url":"{url}","detail":"{detail}"}}}}]}}"#
    )
}

fn image_url_string(url: &str) -> String {
    format!(r#"{{"content":[{{"type":"image_url","image_url":"{url}"}}]}}"#)
}

fn document(source: &str, title: &str) -> String {
    format!(r#"{{"content":[{{"type":"document","title":"{title}","source":{source}}}]}}"#)
}

fn file(filename: &str, file_data: &str) -> String {
    format!(
        r#"{{"content":[{{"type":"file","file":{{"filename":"{filename}","file_data":"{file_data}"}}}}]}}"#
    )
}

#[rstest]
#[case::text_item(plain(r#""hello world""#), EXACT, tokens(2))]
#[case::text_block(plain(r#"{"type":"text","text":"hello world"}"#), EXACT, tokens(2))]
#[case::text_block_without_text(plain(r#"{"type":"text"}"#), EXACT, tokens(0))]
#[case::items_sum(plain(r#""hello",{"type":"text","text":"world"}"#), EXACT, tokens(2))]
#[case::unknown_fields_ignored(
    plain(r#"{"type":"text","text":"hello","cache_control":{"type":"ephemeral"}}"#),
    EXACT,
    tokens(1)
)]
#[case::image_url_string_inline(image_url_string(PNG), EXACT, tokens(85))]
#[case::image_url_string_remote_default(image_url_string(REMOTE), DEFAULT, remote_tokens(250))]
#[case::image_url_string_remote_exact(image_url_string(REMOTE), EXACT, tokens(85))]
#[case::image_url_low(image_url(PNG, "low"), EXACT, tokens(85))]
#[case::image_url_auto(image_url(PNG, "auto"), EXACT, tokens(85))]
#[case::image_url_high_local(image_url(PNG, "high"), EXACT, tokens(255))]
#[case::image_url_high_local_default_mode(image_url(PNG, "high"), DEFAULT, tokens(250))]
#[case::image_url_remote_high_exact(
    image_url(REMOTE, "high"),
    EXACT,
    failed("an image needs its exact dimensions and no local bytes were given")
)]
#[case::image_url_remote_high_default(image_url(REMOTE, "high"), DEFAULT, remote_tokens(250))]
#[case::image_url_remote_high_settings(image_url(REMOTE, "high"), DEFAULT_123, remote_tokens(123))]
#[case::image_url_empty_string(
    image_url_string(""),
    EXACT,
    failed("a string image_url must not be blank")
)]
#[case::image_url_whitespace_string(
    image_url_string("  "),
    EXACT,
    failed("a string image_url must not be blank")
)]
#[case::image_url_missing(plain(r#"{"type":"image_url"}"#), EXACT, Expected::Invalid)]
#[case::image_url_null(
    plain(r#"{"type":"image_url","image_url":null}"#),
    EXACT,
    Expected::Invalid
)]
#[case::image_url_empty_url(
    image_url("", "auto"),
    EXACT,
    failed("an image_url object needs a url")
)]
#[case::image_url_bad_detail(image_url(PNG, "medium"), EXACT, Expected::Invalid)]
#[case::image_base64(
    plain(r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}"#),
    EXACT,
    tokens(85)
)]
#[case::image_source_url_exact(
    plain(r#"{"type":"image","source":{"type":"url","url":"https://example.com/a.png"}}"#),
    EXACT,
    tokens(85)
)]
#[case::image_source_url_default(
    plain(r#"{"type":"image","source":{"type":"url","url":"https://example.com/a.png"}}"#),
    DEFAULT,
    remote_tokens(250)
)]
#[case::image_source_file(
    plain(r#"{"type":"image","source":{"type":"file","file_id":"file-1"}}"#),
    EXACT,
    tokens(85)
)]
#[case::image_source_unsupported(
    plain(r#"{"type":"image","source":{"type":"text","data":"hello"}}"#),
    EXACT,
    failed("the text source is not counted")
)]
#[case::image_missing_source(plain(r#"{"type":"image"}"#), EXACT, Expected::Invalid)]
#[case::document_title_context_text(
    r#"{"content":[{"type":"document","title":"hello","context":"world","source":{"type":"text","media_type":"text/plain","data":"hello world"}}]}"#,
    EXACT,
    tokens(4)
)]
#[case::document_empty_title_context(
    r#"{"content":[{"type":"document","title":"","context":"","source":{"type":"text","data":"hello"}}]}"#,
    EXACT,
    tokens(1)
)]
#[case::document_context(
    r#"{"content":[{"type":"document","context":"world","source":{"type":"text","data":"hello"}}]}"#,
    EXACT,
    tokens(2)
)]
#[case::document_content_string(
    document(r#"{"type":"content","content":"hello world"}"#, ""),
    EXACT,
    tokens(2)
)]
#[case::document_content_list(
    document(
        r#"{"type":"content","content":[{"type":"text","text":"hello"},"world"]}"#,
        ""
    ),
    EXACT,
    tokens(2)
)]
#[case::document_base64(
    document(
        r#"{"type":"base64","media_type":"application/pdf","data":"AAAA"}"#,
        ""
    ),
    EXACT,
    tokens(85)
)]
#[case::document_url_exact(
    document(r#"{"type":"url","url":"https://example.com/a.pdf"}"#, ""),
    EXACT,
    tokens(85)
)]
#[case::document_url_default(
    document(r#"{"type":"url","url":"https://example.com/a.pdf"}"#, ""),
    DEFAULT,
    remote_tokens(250)
)]
#[case::document_missing_source(
    plain(r#"{"type":"document","title":"hello"}"#),
    EXACT,
    Expected::Invalid
)]
#[case::document_text_without_data(
    plain(r#"{"type":"document","source":{"type":"text"}}"#),
    EXACT,
    Expected::Invalid
)]
#[case::document_unknown_source_counts_as_image(
    plain(r#"{"type":"document","source":{"type":"blob"}}"#),
    EXACT,
    tokens(85)
)]
#[case::file_name_and_data(file("hello", PNG), EXACT, tokens(86))]
#[case::file_name_only(file("hello world", ""), EXACT, tokens(2))]
#[case::file_data_only(file("", PNG), EXACT, tokens(85))]
#[case::file_empty_values(file("", ""), EXACT, tokens(0))]
#[case::file_missing(plain(r#"{"type":"file"}"#), EXACT, Expected::Invalid)]
#[case::tool_use_name(plain(r#"{"type":"tool_use","name":"hello world"}"#), EXACT, tokens(2))]
#[case::tool_use_input(
    plain(r#"{"type":"tool_use","input":{"city":"Paris"}}"#),
    EXACT,
    tokens(5)
)]
#[case::tool_use_input_and_caller(
    plain(r#"{"type":"tool_use","input":{"a":1},"caller":{"type":"direct"}}"#),
    EXACT,
    tokens(10)
)]
#[case::tool_use_null_input(plain(r#"{"type":"tool_use","input":null}"#), EXACT, tokens(0))]
#[case::tool_result_string(
    plain(r#"{"type":"tool_result","tool_use_id":"toolu_1","content":"hello world"}"#),
    EXACT,
    tokens(2)
)]
#[case::tool_result_nested(plain(r#"{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"hello"},{"type":"image","source":{"type":"base64","data":"AAAA"}}]}"#), EXACT, tokens(86))]
#[case::tool_result_missing_id(
    plain(r#"{"type":"tool_result","content":"hello"}"#),
    EXACT,
    Expected::Invalid
)]
#[case::tool_result_missing_id_fallback(
    plain(r#"{"type":"tool_result","content":"hello"}"#),
    FALLBACK,
    fallback_tokens(777)
)]
#[case::thinking(
    plain(r#"{"type":"thinking","thinking":"hello world"}"#),
    EXACT,
    tokens(2)
)]
#[case::redacted_thinking(
    plain(r#"{"type":"redacted_thinking","data":"abc"}"#),
    EXACT,
    tokens(0)
)]
#[case::tool_reference(
    plain(r#"{"type":"tool_reference","tool_name":"hello"}"#),
    EXACT,
    tokens(1)
)]
#[case::tool_reference_empty(
    plain(r#"{"type":"tool_reference","tool_name":""}"#),
    EXACT,
    tokens(0)
)]
#[case::unsupported_block(
    plain(r#"{"type":"video_url"}"#),
    EXACT,
    failed("content block type video_url is not counted")
)]
#[case::unsupported_block_fallback(
    plain(r#"{"type":"video_url"}"#),
    FALLBACK,
    fallback_tokens(777)
)]
#[case::fallback_discards_earlier_blocks(plain(&format!(r#""hello world",{{"type":"image_url","image_url":{{"url":"{TRUNCATED}","detail":"high"}}}}"#)), FALLBACK, fallback_tokens(777))]
#[case::fallback_inside_document(document(&format!(r#"{{"type":"content","content":[{{"type":"image_url","image_url":{{"url":"{TRUNCATED}","detail":"high"}}}}]}}"#), "hello"), FALLBACK, fallback_tokens(778))]
#[case::fallback_inside_tool_result(plain(&format!(r#"{{"type":"tool_result","tool_use_id":"toolu_1","content":[{{"type":"image_url","image_url":{{"url":"{TRUNCATED}","detail":"high"}}}}]}}"#)), FALLBACK, fallback_tokens(777))]
#[case::nested_parse_error_keeps_siblings_in_tool_result(plain(r#""hello world",{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"image_url","image_url":{"url":"x","detail":"medium"}}]}"#), FALLBACK, fallback_tokens(779))]
#[case::nested_parse_error_keeps_siblings_in_document(
    document(r#"{"type":"content","content":[{"type":"image_url"}]}"#, "hello"),
    FALLBACK,
    fallback_tokens(778)
)]
#[case::nested_non_list_content_keeps_siblings(
    plain(r#""hello world",{"type":"tool_result","tool_use_id":"toolu_1","content":5}"#),
    FALLBACK,
    fallback_tokens(779)
)]
#[case::nested_non_list_content_without_fallback_fails(
    plain(r#"{"type":"tool_result","tool_use_id":"toolu_1","content":5}"#),
    EXACT,
    failed("a nested content list could not be read")
)]
#[case::nested_parse_error_without_fallback_fails(
    plain(r#"{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"image_url"}]}"#),
    EXACT,
    failed("a nested content list could not be read")
)]
#[case::depth_at_limit(support::nested_documents(31), EXACT, tokens(1))]
#[case::depth_past_limit(
    support::nested_documents(32),
    EXACT,
    failed("the content nesting exceeds 32 levels")
)]
#[case::depth_past_limit_fallback(support::nested_documents(32), FALLBACK, fallback_tokens(777))]
#[case::overflow(plain(&image_twice()) , MAX_COUNT, failed("the token count does not fit in a machine word"))]
#[case::overflow_fallback(plain(&image_twice()), MAX_COUNT_FALLBACK, fallback_tokens(7))]
#[case::invalid_json("not json", EXACT, Expected::Invalid)]
#[case::content_item_number(r#"{"content":[5]}"#, EXACT, Expected::Invalid)]
#[case::image_unknown_source_counts_as_image(
    plain(r#"{"type":"image","source":{"type":"blob"}}"#),
    EXACT,
    tokens(85)
)]
#[case::image_url_number(
    plain(r#"{"type":"image_url","image_url":5}"#),
    EXACT,
    Expected::Invalid
)]
#[case::nested_unsupported_block(
    plain(r#"{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"video_url"}]}"#),
    EXACT,
    failed("a nested content block type is not counted")
)]
#[case::nested_unsupported_block_fallback(
    plain(r#"{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"video_url"}]}"#),
    FALLBACK,
    fallback_tokens(777)
)]
#[tokio::test]
async fn media_tokens_media_token_tests_behavior(
    #[case] input: String,
    #[case] options: CaseOptions,
    #[case] expected: Expected,
) {
    let fixture = support::words_fixture();
    let settings = support::settings(&options);
    match (support::count(&fixture, &input, &settings).await, expected) {
        (
            Ok(counted),
            Expected::Counted {
                tokens,
                remote_warnings,
                fallback_warnings,
            },
        ) => {
            assert_eq!(counted.tokens, tokens);
            let remote = counted
                .warnings
                .iter()
                .filter(|warning| matches!(warning, Warning::RemoteImageDefaultCount))
                .count();
            assert_eq!(remote, remote_warnings);
            let fallbacks = counted
                .warnings
                .iter()
                .filter(|warning| matches!(warning, Warning::FallbackTokenCount { .. }))
                .count();
            assert_eq!(fallbacks, fallback_warnings);
        }
        (Err(error), Expected::Failed(message)) => assert_eq!(error.to_string(), message),
        (Err(_), Expected::Invalid) => {}
        (result, unexpected) => panic!("unexpected {result:?} for {unexpected:?}"),
    }
}

fn image_twice() -> String {
    let block = format!(r#"{{"type":"image_url","image_url":{{"url":"{PNG}","detail":"auto"}}}}"#);
    format!("{block},{block}")
}
