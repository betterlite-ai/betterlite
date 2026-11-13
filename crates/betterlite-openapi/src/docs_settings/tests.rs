#![cfg(test)]

use rstest::rstest;

use crate::docs_settings::{DocsEnv, DocsUrls};

fn empty_env() -> DocsEnv {
    DocsEnv {
        docs_url: None,
        redoc_url: None,
        openapi_url: None,
        no_docs: None,
        no_redoc: None,
        no_openapi: None,
        root_redirect_url: None,
    }
}

fn text(value: Option<&str>) -> Option<String> {
    value.map(str::to_owned)
}

#[test]
fn defaults_apply_when_nothing_is_set() {
    assert_eq!(
        empty_env().resolve(),
        DocsUrls {
            docs_url: Some("/".to_owned()),
            redoc_url: Some("/redoc".to_owned()),
            openapi_url: Some("/openapi.json".to_owned()),
            root_redirect_url: None,
        }
    );
}

#[test]
fn explicit_urls_win_over_the_defaults() {
    let mut env = empty_env();
    env.docs_url = text(Some("/docs"));
    env.redoc_url = text(Some("/catalog"));
    env.openapi_url = text(Some("/spec.json"));
    let urls = env.resolve();
    assert_eq!(urls.docs_url.as_deref(), Some("/docs"));
    assert_eq!(urls.redoc_url.as_deref(), Some("/catalog"));
    assert_eq!(urls.openapi_url.as_deref(), Some("/spec.json"));
}

#[test]
fn explicit_urls_win_over_a_true_flag() {
    let mut env = empty_env();
    env.docs_url = text(Some("/docs"));
    env.redoc_url = text(Some("/catalog"));
    env.openapi_url = text(Some("/spec.json"));
    env.no_docs = text(Some("true"));
    env.no_redoc = text(Some("true"));
    env.no_openapi = text(Some("true"));
    let urls = env.resolve();
    assert_eq!(urls.docs_url.as_deref(), Some("/docs"));
    assert_eq!(urls.redoc_url.as_deref(), Some("/catalog"));
    assert_eq!(urls.openapi_url.as_deref(), Some("/spec.json"));
}

#[test]
fn an_empty_explicit_url_counts_as_unset() {
    let mut env = empty_env();
    env.docs_url = text(Some(""));
    env.redoc_url = text(Some(""));
    env.openapi_url = text(Some(""));
    let urls = env.resolve();
    assert_eq!(urls.docs_url.as_deref(), Some("/"));
    assert_eq!(urls.redoc_url.as_deref(), Some("/redoc"));
    assert_eq!(urls.openapi_url.as_deref(), Some("/openapi.json"));
}

#[test]
fn an_empty_docs_url_with_no_docs_true_gives_none() {
    let mut env = empty_env();
    env.docs_url = text(Some(""));
    env.no_docs = text(Some("true"));
    assert_eq!(env.resolve().docs_url, None);
}

#[rstest]
#[case::true_word(Some("true"))]
#[case::upper_case(Some("TRUE"))]
#[case::padded(Some(" true "))]
fn a_true_no_docs_flag_removes_the_page(#[case] flag: Option<&str>) {
    let mut env = empty_env();
    env.no_docs = text(flag);
    assert_eq!(env.resolve().docs_url, None);
}

#[rstest]
#[case::true_word(Some("true"))]
#[case::upper_case(Some("True"))]
#[case::padded(Some("\ttrue\n"))]
fn a_true_no_redoc_flag_removes_the_page(#[case] flag: Option<&str>) {
    let mut env = empty_env();
    env.no_redoc = text(flag);
    assert_eq!(env.resolve().redoc_url, None);
}

#[rstest]
#[case::true_word(Some("true"))]
#[case::upper_case(Some("tRuE"))]
#[case::padded(Some("  TRUE  "))]
fn a_true_no_openapi_flag_removes_the_document(#[case] flag: Option<&str>) {
    let mut env = empty_env();
    env.no_openapi = text(flag);
    assert_eq!(env.resolve().openapi_url, None);
}

#[rstest]
#[case::yes(Some("yes"))]
#[case::one(Some("1"))]
#[case::on(Some("on"))]
#[case::false_word(Some("false"))]
#[case::empty(Some(""))]
#[case::unknown(Some(" trueish "))]
#[case::unset(None)]
fn other_flag_text_does_not_disable_a_page(#[case] flag: Option<&str>) {
    let mut env = empty_env();
    env.no_docs = text(flag);
    env.no_redoc = text(flag);
    env.no_openapi = text(flag);
    let urls = env.resolve();
    assert_eq!(urls.docs_url.as_deref(), Some("/"));
    assert_eq!(urls.redoc_url.as_deref(), Some("/redoc"));
    assert_eq!(urls.openapi_url.as_deref(), Some("/openapi.json"));
}

#[rstest]
#[case::unset(None, None)]
#[case::empty(Some(""), Some(""))]
#[case::path(Some("/ui"), Some("/ui"))]
fn root_redirect_url_is_passed_through(
    #[case] configured: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut env = empty_env();
    env.root_redirect_url = text(configured);
    assert_eq!(env.resolve().root_redirect_url.as_deref(), expected);
}
