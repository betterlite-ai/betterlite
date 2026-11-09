#![cfg(test)]

use std::fs;

use rstest::rstest;

use crate::docs_pages::{
    SwaggerPageInputs, oauth2_redirect_page, redoc_page, swagger_asset, swagger_ui_page,
};

const SWAGGER_ROOT: &str = include_str!("fixtures/swagger-root.html");
const SWAGGER_ROOT_PATH: &str = include_str!("fixtures/swagger-root-path.html");
const SWAGGER_NO_OAUTH: &str = include_str!("fixtures/swagger-no-oauth.html");
const REDOC_ROOT: &str = include_str!("fixtures/redoc-root.html");
const REDOC_ROOT_PATH: &str = include_str!("fixtures/redoc-root-path.html");
const OAUTH2_REDIRECT: &str = include_str!("fixtures/oauth2-redirect.html");

fn inputs(
    title: &str,
    openapi_url: &str,
    oauth2_redirect_url: Option<&str>,
    swagger_base: &str,
) -> SwaggerPageInputs {
    SwaggerPageInputs {
        title: title.to_owned(),
        openapi_url: openapi_url.to_owned(),
        oauth2_redirect_url: oauth2_redirect_url.map(str::to_owned),
        swagger_base: swagger_base.to_owned(),
    }
}

#[test]
fn swagger_page_matches_fastapi_without_root_path() {
    let page = swagger_ui_page(&inputs(
        "LiteLLM API - Swagger UI",
        "/openapi.json",
        Some("/docs/oauth2-redirect"),
        "/swagger",
    ));
    assert_eq!(page, SWAGGER_ROOT);
}

#[test]
fn swagger_page_matches_fastapi_with_root_path() {
    let page = swagger_ui_page(&inputs(
        "LiteLLM API - Swagger UI",
        "/api/openapi.json",
        Some("/api/docs/oauth2-redirect"),
        "/api/swagger",
    ));
    assert_eq!(page, SWAGGER_ROOT_PATH);
}

#[test]
fn swagger_page_without_oauth2_redirect_matches_fastapi() {
    let page = swagger_ui_page(&inputs(
        "LiteLLM API - Swagger UI",
        "/openapi.json",
        None,
        "/swagger",
    ));
    assert_eq!(page, SWAGGER_NO_OAUTH);
}

#[test]
fn redoc_page_matches_fastapi_without_root_path() {
    assert_eq!(
        redoc_page("LiteLLM API - ReDoc", "/openapi.json"),
        REDOC_ROOT
    );
}

#[test]
fn redoc_page_matches_fastapi_with_root_path() {
    assert_eq!(
        redoc_page("LiteLLM API - ReDoc", "/api/openapi.json"),
        REDOC_ROOT_PATH
    );
}

#[test]
fn oauth2_redirect_page_matches_fastapi() {
    assert_eq!(oauth2_redirect_page(), OAUTH2_REDIRECT);
}

#[rstest]
#[case::bundle("swagger-ui-bundle.js", "text/javascript")]
#[case::stylesheet("swagger-ui.css", "text/css")]
#[case::png("favicon.png", "image/png")]
#[case::ico("favicon.ico", "image/vnd.microsoft.icon")]
fn swagger_asset_returns_the_embedded_file(#[case] name: &str, #[case] media_type: &str) {
    let asset = swagger_asset(name).unwrap();
    assert_eq!(asset.content_type, media_type);
    let served = fs::read(format!(
        "{}/assets/swagger/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert_eq!(asset.bytes, served);
}

#[rstest]
#[case::parent("../swagger-ui.css")]
#[case::nested("a/swagger-ui.css")]
#[case::empty("")]
#[case::uppercase("SWAGGER-UI.CSS")]
#[case::trailing_space("swagger-ui.css ")]
#[case::dot(".")]
#[case::dotdot("..")]
fn swagger_asset_rejects_other_names(#[case] name: &str) {
    assert!(swagger_asset(name).is_none());
}
