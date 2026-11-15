#![cfg(test)]

use rstest::rstest;

use super::{DASHSCOPE_RERANK_URL, resolve_rerank_base};

#[rstest]
#[case::the_plain_chat_base(
    "https://dashscope.aliyuncs.com/compatible-mode/v1",
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::the_international_chat_base(
    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    "https://dashscope-intl.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::the_bare_aliyun_host(
    "https://aliyuncs.com/compatible-mode/v1",
    "https://aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::a_trailing_slash(
    "https://dashscope.aliyuncs.com/compatible-mode/v1///",
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::an_upper_case_host(
    "https://DASHSCOPE.ALIYUNCS.COM/compatible-mode/v1",
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::the_scheme_and_the_port(
    "http://dashscope.aliyuncs.com:8443/compatible-mode/v1",
    "http://dashscope.aliyuncs.com:8443/compatible-api/v1/reranks"
)]
fn remaps_a_chat_shaped_aliyun_base_to_the_same_host_rerank_route(
    #[case] base: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        resolve_rerank_base(Some(base), "", DASHSCOPE_RERANK_URL),
        expected
    );
}

#[rstest]
#[case::a_rerank_url("https://example.test/reranks")]
#[case::a_bare_host("https://example.test")]
#[case::a_path_that_is_not_chat_shaped("https://dashscope.aliyuncs.com/compatible-mode/v2")]
#[case::a_lookalike_host("https://evil-aliyuncs.com/compatible-mode/v1")]
#[case::no_scheme("dashscope.aliyuncs.com/compatible-mode/v1")]
#[case::empty("")]
fn keeps_an_explicit_base_that_is_not_chat_shaped(#[case] base: &str) {
    assert_eq!(
        resolve_rerank_base(Some(base), "", DASHSCOPE_RERANK_URL),
        base
    );
}

#[rstest]
#[case::a_chat_shaped_base(Some("https://dashscope.aliyuncs.com/compatible-mode/v1"))]
#[case::no_base(None)]
fn lets_the_environment_value_win(#[case] base: Option<&str>) {
    assert_eq!(
        resolve_rerank_base(base, "https://my.rerank.test/v1", DASHSCOPE_RERANK_URL),
        "https://my.rerank.test/v1"
    );
}

#[test]
fn falls_back_to_the_remap_then_the_default() {
    assert_eq!(
        resolve_rerank_base(
            Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
            "",
            DASHSCOPE_RERANK_URL
        ),
        "https://dashscope-intl.aliyuncs.com/compatible-api/v1/reranks"
    );
    assert_eq!(
        resolve_rerank_base(None, "", DASHSCOPE_RERANK_URL),
        "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
    );
}
