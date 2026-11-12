use betterlite_openai::files::records::FileContentRequest;
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::file_contracts::base::BaseConfig as _;
use crate::api::file_contracts::config::{BaseFilesConfig as _, CompleteUrlRequest, FileParams};
use crate::api::file_contracts::error::FileContractError;
use crate::manus_files::{MANUS_API_BASE, MANUS_API_BASE_ENV, ManusFilesConfig};

use super::support::{NOW, config, environment, globals, params, plain_config};

fn complete_url(
    config: &ManusFilesConfig,
    api_base: Option<&str>,
) -> Result<String, FileContractError> {
    let optional_params = FileParams::new();
    let litellm_params = FileParams::new();
    config.get_complete_url(CompleteUrlRequest {
        api_base,
        api_key: None,
        model: "",
        optional_params: &optional_params,
        litellm_params: &litellm_params,
    })
}

#[rstest]
#[case::explicit(Some("http://explicit:1"), None, &[], "http://explicit:1/v1/files")]
#[case::explicit_v1(Some("http://explicit:1/v1"), None, &[], "http://explicit:1/v1/files")]
#[case::explicit_v1_slash(Some("http://explicit:1/v1/"), None, &[], "http://explicit:1/v1/files")]
#[case::explicit_many_slashes(Some("http://explicit:1///"), None, &[], "http://explicit:1/v1/files")]
#[case::explicit_other_path(Some("http://explicit:1/v1beta"), None, &[], "http://explicit:1/v1beta/v1/files")]
#[case::global(None, Some("http://global:2"), &[], "http://global:2/v1/files")]
#[case::environment(None, None, &[(MANUS_API_BASE_ENV, "http://env:3/v1")], "http://env:3/v1/files")]
#[case::empty_explicit(Some(""), Some("http://global:2"), &[], "http://global:2/v1/files")]
#[case::empty_global(Some(""), Some(""), &[(MANUS_API_BASE_ENV, "http://env:3")], "http://env:3/v1/files")]
#[case::empty_environment(None, None, &[(MANUS_API_BASE_ENV, "")], "https://api.manus.im/v1/files")]
#[case::nothing(None, None, &[], "https://api.manus.im/v1/files")]
fn the_files_url_follows_the_base_chain(
    #[case] explicit: Option<&str>,
    #[case] global: Option<&str>,
    #[case] environment_pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let config = config(globals(global, None), environment(environment_pairs), NOW);
    assert_eq!(
        complete_url(&config, explicit).expect("the base resolves"),
        expected
    );
}

#[test]
fn the_default_base_is_the_documented_constant() {
    assert_eq!(MANUS_API_BASE, "https://api.manus.im");
    assert_eq!(
        complete_url(&plain_config(NOW), None).expect("the default resolves"),
        format!("{MANUS_API_BASE}/v1/files")
    );
}

#[rstest]
#[case::plain("file-1", "file-1")]
#[case::space("a b", "a%20b")]
#[case::slash("a/b", "a%2Fb")]
#[case::query("x?y#z", "x%3Fy%23z")]
#[case::unicode("\u{fc}", "%C3%BC")]
fn retrieve_and_delete_share_the_encoded_file_url(#[case] file_id: &str, #[case] encoded: &str) {
    let config = plain_config(NOW);
    let empty = FileParams::new();
    let (retrieve, retrieve_params) = config
        .transform_retrieve_file_request(file_id, &empty, &empty)
        .expect("the file id is a path segment");
    let (delete, delete_params) = config
        .transform_delete_file_request(file_id, &empty, &empty)
        .expect("the file id is a path segment");
    let expected = format!("{MANUS_API_BASE}/v1/files/{encoded}");
    assert_eq!(retrieve, expected);
    assert_eq!(delete, expected);
    assert!(retrieve_params.is_empty());
    assert!(delete_params.is_empty());
}

#[test]
fn file_urls_use_the_litellm_params_api_base() {
    let config = plain_config(NOW);
    let litellm_params = params(r#"{"api_base":"http://host/v1"}"#);
    let empty = FileParams::new();
    let (url, _) = config
        .transform_retrieve_file_request("file-1", &empty, &litellm_params)
        .expect("the base resolves");
    assert_eq!(url, "http://host/v1/files/file-1");
}

#[rstest]
#[case::empty("", "file_id is required")]
#[case::dot(".", "file_id cannot be a dot path segment")]
#[case::dot_dot("..", "file_id cannot be a dot path segment")]
fn a_file_id_that_is_not_a_path_segment_is_refused(#[case] file_id: &str, #[case] cause: &str) {
    let error = plain_config(NOW)
        .transform_retrieve_file_request(file_id, &FileParams::new(), &FileParams::new())
        .expect_err("the id is not a path segment");
    let FileContractError::Upstream(inner) = error else {
        panic!("the module error becomes the upstream error type");
    };
    assert_eq!(inner.status_code, 500);
    assert!(
        inner
            .message
            .contains("cannot be written as a URL path segment")
    );
    assert!(inner.message.ends_with(&format!(": {cause}")));
}

#[test]
fn a_non_text_api_base_parameter_is_refused() {
    let error = plain_config(NOW)
        .transform_retrieve_file_request("file-1", &FileParams::new(), &params(r#"{"api_base":7}"#))
        .expect_err("the base is not text");
    let FileContractError::Upstream(inner) = error else {
        panic!("the module error becomes the upstream error type");
    };
    assert_eq!(inner.message, "the api_base parameter must be text");
}

#[rstest]
#[case::plain("file-1", "file-1")]
#[case::space("a b", "a%20b")]
#[case::unicode("\u{fc}", "%C3%BC")]
fn the_content_url_appends_the_content_path(#[case] file_id: &str, #[case] encoded: &str) {
    let request = FileContentRequest {
        file_id: String::from(file_id),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    let (url, params) = plain_config(NOW)
        .transform_file_content_request(&request, &FileParams::new(), &FileParams::new())
        .expect("the file id is a path segment");
    assert_eq!(url, format!("{MANUS_API_BASE}/v1/files/{encoded}/content"));
    assert!(params.is_empty());
}
