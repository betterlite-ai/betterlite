#![cfg(test)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::thread;

use aws_lc_rs::digest::{SHA256, digest};
use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use betterlite_openai::error::ProviderError;
use betterlite_prompts::arguments::json::{Json, Object};
use chrono::{DateTime, Utc};
use http::HeaderMap;
use http::header::HeaderValue;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::oci_common::base_url::{OCI_REGION_PATTERN, get_oci_base_url};
use crate::oci_common::credentials::{
    DEFAULT_OCI_REGION, OciCredentials, OciEnvValues, OciExplicitParams, resolve_oci_credentials,
};
use crate::oci_common::environment::{
    DEFAULT_LITELLM_VERSION, OciEnvironment, validate_oci_environment,
};
use crate::oci_common::schema::refs::resolve_oci_schema_refs;
use crate::oci_common::schema::{
    OCI_JSON_TO_PYTHON_TYPES, enrich_cohere_description, resolve_oci_schema_anyof,
    sanitize_oci_schema,
};
use crate::oci_common::signing::sign_oci_request;
use crate::oci_common::{Clock, OciError, OciServingMode};

const TEST_KEY: &str = include_str!("fixtures/test-private.pem");

const TEST_PUBLIC_KEY: &str = include_str!("fixtures/test-public.pem");
const TEST_PKCS1_KEY: &str = include_str!("fixtures/test-private-pkcs1.pem");

const TEST_EC_KEY: &str = include_str!("fixtures/test-ec-private.pem");

const TEST_SMALL_KEY: &str = include_str!("fixtures/test-small-private.pem");

const TEST_API_BASE: &str =
    "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat?x=1";
const TEST_PATH: &str = "/20231130/actions/chat";
const TEST_HOST: &str = "inference.generativeai.us-ashburn-1.oci.oraclecloud.com";
const TEST_SIGNATURE: &str = "QzgciPkZkWV6nsJ3ZyqV1Si2e94ljZvBuzqMTySeMJXhOFfnnMsqiDzr96QdoxKKEzSdCUqz+y6ujmjY96CwWePwL2lPun8jCyEtU6VuwErMLsCM/DX4GA0rTQNKLCTKjHAuh5UjkHVxVFPeTC/so9UjJl7/3ORTVoGlO0dfcujck1S61OHNENA0iJcHNlfRqQxgaxU00VrkIY4AodUI6kkFq21azDF/pg5vX1iZYSh1uGgqWxscespqAXvbd+FUVNDDmmjT7d5NUxDEhONJN17a+D8v7R/KCwhodFO/QMAdvxcYZMz1jX8+ukrnjC4xBD7uTfha75vzrNItS3OScA==";
const FIXED_SECONDS: i64 = 1_730_882_977;

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn fixed_clock() -> FixedClock {
    FixedClock(DateTime::<Utc>::from_timestamp(FIXED_SECONDS, 0).unwrap())
}

fn signing_credentials(key: &str) -> OciCredentials {
    OciCredentials {
        oci_region: DEFAULT_OCI_REGION.to_owned(),
        oci_user: Some("ocid1.user.oc1..aaaa".to_owned()),
        oci_fingerprint: Some("aa:bb:cc".to_owned()),
        oci_tenancy: Some("ocid1.tenancy.oc1..bbbb".to_owned()),
        oci_key: Some(SecretString::from(key.to_owned())),
        oci_key_file: None,
        oci_compartment_id: None,
    }
}

fn parse_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn temp_path(name: &str) -> PathBuf {
    env::temp_dir().join(format!("betterlite-oci-{}-{name}.pem", process::id()))
}

fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers.get(name).unwrap().to_str().unwrap().to_owned()
}

fn assert_signature_verifies(headers: &HeaderMap, path: &str) {
    let signing_string = format!(
        "date: {}\n(request-target): post {path}\nhost: {}\ncontent-length: {}\ncontent-type: {}\nx-content-sha256: {}",
        header_text(headers, "date"),
        header_text(headers, "host"),
        header_text(headers, "content-length"),
        header_text(headers, "content-type"),
        header_text(headers, "x-content-sha256"),
    );
    let authorization = header_text(headers, "authorization");
    let encoded = authorization
        .split("signature=\"")
        .nth(1)
        .unwrap()
        .trim_end_matches('"');
    let signature = STANDARD.decode(encoded).unwrap();
    let public = pem::parse(TEST_PUBLIC_KEY).unwrap();
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public.contents())
        .verify(signing_string.as_bytes(), &signature)
        .unwrap();
}

fn deepest_value(mut current: &Json) -> &Json {
    while let Some(inner) = current
        .as_object()
        .and_then(|object| object.get("properties"))
        .filter(|inner| inner.as_object().is_some())
    {
        current = inner;
    }
    current
}

#[test]
fn resolve_oci_credentials_prefers_explicit_values() {
    let explicit = OciExplicitParams {
        oci_region: Some(String::new()),
        oci_user: Some("explicit-user".to_owned()),
        oci_fingerprint: None,
        oci_tenancy: Some("explicit-tenancy".to_owned()),
        oci_key: Some(SecretString::from("explicit-key")),
        oci_key_file: None,
        oci_compartment_id: None,
    };
    let env = OciEnvValues {
        oci_region: Some(String::new()),
        oci_user: Some("env-user".to_owned()),
        oci_fingerprint: Some("env-fingerprint".to_owned()),
        oci_tenancy: Some("env-tenancy".to_owned()),
        oci_key: Some(SecretString::from("env-key")),
        oci_key_file: Some("env-key-file".to_owned()),
        oci_compartment_id: Some("env-compartment".to_owned()),
    };
    let credentials = resolve_oci_credentials(&explicit, &env);
    assert_eq!(credentials.oci_region, DEFAULT_OCI_REGION);
    assert_eq!(credentials.oci_user.as_deref(), Some("explicit-user"));
    assert_eq!(
        credentials.oci_fingerprint.as_deref(),
        Some("env-fingerprint")
    );
    assert_eq!(credentials.oci_tenancy.as_deref(), Some("explicit-tenancy"));
    assert_eq!(
        credentials
            .oci_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("explicit-key")
    );
    assert_eq!(credentials.oci_key_file.as_deref(), Some("env-key-file"));
    assert_eq!(
        credentials.oci_compartment_id.as_deref(),
        Some("env-compartment")
    );
}

#[test]
fn resolve_oci_credentials_defaults_the_region_and_leaves_the_rest_absent() {
    let credentials =
        resolve_oci_credentials(&OciExplicitParams::default(), &OciEnvValues::default());
    assert_eq!(credentials.oci_region, "us-ashburn-1");
    assert!(credentials.oci_user.is_none());
    assert!(credentials.oci_fingerprint.is_none());
    assert!(credentials.oci_tenancy.is_none());
    assert!(credentials.oci_key.is_none());
    assert!(credentials.oci_key_file.is_none());
    assert!(credentials.oci_compartment_id.is_none());
}

#[test]
fn get_oci_base_url_builds_the_region_url() {
    let url = get_oci_base_url("eu-frankfurt-1", None).unwrap();
    assert_eq!(
        url,
        "https://inference.generativeai.eu-frankfurt-1.oci.oraclecloud.com"
    );
}

#[test]
fn get_oci_base_url_ignores_an_empty_api_base() {
    let url = get_oci_base_url("eu-frankfurt-1", Some("")).unwrap();
    assert_eq!(
        url,
        "https://inference.generativeai.eu-frankfurt-1.oci.oraclecloud.com"
    );
}

#[rstest]
#[case::action_with_slash("https://host/20231130/actions/chat/", "https://host")]
#[case::action_without_slash("https://host/base/20231130/actions/embedText", "https://host/base")]
#[case::plain_base("https://host/base", "https://host/base")]
#[case::nested_name(
    "https://host/x/20231130/actions/a/b",
    "https://host/x/20231130/actions/a/b"
)]
#[case::query_suffix(
    "https://host/20231130/actions/chat?x=1",
    "https://host/20231130/actions/chat?x=1"
)]
#[case::trailing_slashes("https://host///", "https://host")]
#[case::two_slashes_after_action(
    "https://host/20231130/actions/chat//",
    "https://host/20231130/actions/chat"
)]
fn get_oci_base_url_strips_the_action_path(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(
        get_oci_base_url("us-ashburn-1", Some(api_base)).unwrap(),
        expected
    );
}

#[rstest]
#[case::ashburn("us-ashburn-1")]
#[case::two_chars("ab")]
#[case::maximum_length("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
fn get_oci_base_url_accepts_valid_regions(#[case] region: &str) {
    assert_eq!(
        get_oci_base_url(region, None).unwrap(),
        format!("https://inference.generativeai.{region}.oci.oraclecloud.com")
    );
}

#[rstest]
#[case::too_short("a")]
#[case::too_long("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
#[case::uppercase("US-ASHBURN-1")]
#[case::underscore("us_ashburn")]
#[case::trailing_newline("us-ashburn-1\n")]
fn get_oci_base_url_rejects_invalid_regions(#[case] region: &str) {
    let error = get_oci_base_url(region, None).unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("Invalid OCI region"));
    assert!(error.message.contains(OCI_REGION_PATTERN));
}

#[test]
fn validate_oci_environment_adds_the_default_headers() {
    let mut headers = HeaderMap::new();
    validate_oci_environment(&mut headers, &OciEnvironment::default()).unwrap();
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    assert_eq!(
        headers.get("user-agent").unwrap().to_str().unwrap(),
        format!("litellm/{DEFAULT_LITELLM_VERSION}")
    );
}

#[test]
fn validate_oci_environment_keeps_existing_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("text/plain"));
    headers.insert("user-agent", HeaderValue::from_static("custom/9"));
    validate_oci_environment(&mut headers, &OciEnvironment::default()).unwrap();
    assert_eq!(headers.get("content-type").unwrap(), "text/plain");
    assert_eq!(headers.get("user-agent").unwrap(), "custom/9");
}

#[test]
fn validate_oci_environment_uses_the_constructor_version() {
    let mut headers = HeaderMap::new();
    let environment = OciEnvironment {
        version: "9.9.9".to_owned(),
    };
    validate_oci_environment(&mut headers, &environment).unwrap();
    assert_eq!(headers.get("user-agent").unwrap(), "litellm/9.9.9");
}

#[test]
fn sign_oci_request_matches_the_litellm_header_set() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let request = Json::Object(Object::new());
    let body = sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(body, b"{}");
    assert_eq!(
        header_text(&headers, "authorization"),
        format!(
            "Signature version=\"1\",keyId=\"ocid1.tenancy.oc1..bbbb/ocid1.user.oc1..aaaa/aa:bb:cc\",algorithm=\"rsa-sha256\",headers=\"date (request-target) host content-length content-type x-content-sha256\",signature=\"{TEST_SIGNATURE}\""
        )
    );
    assert_eq!(
        header_text(&headers, "date"),
        "Wed, 06 Nov 2024 08:49:37 GMT"
    );
    assert_eq!(header_text(&headers, "host"), TEST_HOST);
    assert_eq!(header_text(&headers, "content-length"), "2");
    assert_eq!(header_text(&headers, "content-type"), "application/json");
    assert_eq!(
        header_text(&headers, "x-content-sha256"),
        "RBNvo1WzZ4oRRq0W9+hknpT7T8If536DEMBg9hyq/4o="
    );
}

#[test]
fn sign_oci_request_keeps_the_raw_host_and_default_port() {
    let mut headers = HeaderMap::new();
    let request = Json::Object(Object::new());
    sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        "https://Inference.Example.com:443/20231130/actions/chat",
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(header_text(&headers, "host"), "Inference.Example.com:443");
    assert_signature_verifies(&headers, "/20231130/actions/chat");
}

#[test]
fn sign_oci_request_keeps_the_raw_path() {
    let mut headers = HeaderMap::new();
    let request = Json::Object(Object::new());
    sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        "https://host/20231130/actions/../actions/chat?x=1",
        &fixed_clock(),
    )
    .unwrap();
    assert_signature_verifies(&headers, "/20231130/actions/../actions/chat");
}

#[test]
fn sign_oci_request_signs_a_string_that_verifies() {
    let mut headers = HeaderMap::new();
    let request = Json::Object(Object::new());
    sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert_signature_verifies(&headers, TEST_PATH);
}

#[test]
fn sign_oci_request_keeps_a_caller_content_type() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("text/plain"));
    let request = Json::Object(Object::new());
    sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(header_text(&headers, "content-type"), "text/plain");
    assert_signature_verifies(&headers, TEST_PATH);
}

#[test]
fn sign_oci_request_keeps_a_mixed_case_content_type() {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("text/plain"));
    let request = Json::Object(Object::new());
    sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(header_text(&headers, "content-type"), "text/plain");
    assert_signature_verifies(&headers, TEST_PATH);
}

#[test]
fn sign_oci_request_rejects_a_malformed_api_base() {
    let request = Json::Object(Object::new());
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &signing_credentials(TEST_KEY),
        &request,
        "not a url",
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(error.message.starts_with("the OCI api base is not a URL"));
}

#[test]
fn sign_oci_request_signs_the_serialized_body_once() {
    let request = parse_json(r#"{"b": [true, null, 1.5], "a": "x"}"#);
    let mut headers = HeaderMap::new();
    let body = sign_oci_request(
        &mut headers,
        &signing_credentials(TEST_KEY),
        &request,
        "https://host/path",
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(body, serde_json::to_vec(&request).unwrap());
    assert_eq!(
        header_text(&headers, "content-length"),
        body.len().to_string()
    );
    assert_eq!(
        header_text(&headers, "x-content-sha256"),
        STANDARD.encode(digest(&SHA256, &body).as_ref())
    );
}

#[test]
fn sign_oci_request_accepts_an_escaped_inline_key() {
    let escaped = TEST_KEY.replace('\n', "\\n");
    let request = Json::Object(Object::new());
    let mut plain_headers = HeaderMap::new();
    sign_oci_request(
        &mut plain_headers,
        &signing_credentials(TEST_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    let mut escaped_headers = HeaderMap::new();
    sign_oci_request(
        &mut escaped_headers,
        &signing_credentials(&escaped),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(
        plain_headers.get("authorization"),
        escaped_headers.get("authorization")
    );
}

#[test]
fn sign_oci_request_accepts_a_pkcs1_key() {
    let request = Json::Object(Object::new());
    let mut pkcs8_headers = HeaderMap::new();
    sign_oci_request(
        &mut pkcs8_headers,
        &signing_credentials(TEST_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    let mut pkcs1_headers = HeaderMap::new();
    sign_oci_request(
        &mut pkcs1_headers,
        &signing_credentials(TEST_PKCS1_KEY),
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert_eq!(
        pkcs8_headers.get("authorization"),
        pkcs1_headers.get("authorization")
    );
}

#[test]
fn sign_oci_request_prefers_the_inline_key_over_the_file() {
    let mut credentials = signing_credentials(TEST_KEY);
    credentials.oci_key_file = Some("/definitely/missing/oci-key.pem".to_owned());
    let request = Json::Object(Object::new());
    let mut headers = HeaderMap::new();
    sign_oci_request(
        &mut headers,
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    assert!(headers.contains_key("authorization"));
}

#[test]
fn sign_oci_request_rejects_missing_credentials() {
    let credentials = OciCredentials {
        oci_region: DEFAULT_OCI_REGION.to_owned(),
        oci_user: Some("ocid1.user.oc1..aaaa".to_owned()),
        oci_fingerprint: Some("aa:bb:cc".to_owned()),
        oci_tenancy: None,
        oci_key: Some(SecretString::from(TEST_KEY)),
        oci_key_file: None,
        oci_compartment_id: None,
    };
    let request = Json::Object(Object::new());
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 401);
    assert!(
        error
            .message
            .starts_with("Missing required OCI credentials")
    );
}

#[test]
fn sign_oci_request_rejects_a_missing_key_source() {
    let mut credentials = signing_credentials(TEST_KEY);
    credentials.oci_key = None;
    let request = Json::Object(Object::new());
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 401);
}

#[test]
fn sign_oci_request_reports_a_missing_key_file() {
    let path = temp_path("missing");
    let mut credentials = signing_credentials(TEST_KEY);
    credentials.oci_key = None;
    credentials.oci_key_file = Some(path.display().to_string());
    let request = Json::Object(Object::new());
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert_eq!(
        error.message,
        format!("Private key file not found: {}", path.display())
    );
}

#[test]
fn sign_oci_request_reports_an_unreadable_key_file() {
    let path = env::temp_dir();
    let mut credentials = signing_credentials(TEST_KEY);
    credentials.oci_key = None;
    credentials.oci_key_file = Some(path.display().to_string());
    let request = Json::Object(Object::new());
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Failed to read private key file '")
    );
    assert!(error.message.contains(&path.display().to_string()));
}

#[test]
fn sign_oci_request_reports_an_empty_key_file() {
    let path = temp_path("empty");
    fs::write(&path, "  \n").unwrap();
    let mut credentials = signing_credentials(TEST_KEY);
    credentials.oci_key = None;
    credentials.oci_key_file = Some(path.display().to_string());
    let request = Json::Object(Object::new());
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    fs::remove_file(&path).unwrap();
    assert_eq!(error.status, 500);
    assert_eq!(
        error.message,
        format!("Private key file is empty: {}", path.display())
    );
}

#[test]
fn sign_oci_request_reads_the_key_from_the_file() {
    let path = temp_path("valid");
    fs::write(&path, format!("\n{TEST_KEY}\n")).unwrap();
    let mut credentials = signing_credentials(TEST_KEY);
    credentials.oci_key = None;
    credentials.oci_key_file = Some(path.display().to_string());
    let request = Json::Object(Object::new());
    let mut headers = HeaderMap::new();
    sign_oci_request(
        &mut headers,
        &credentials,
        &request,
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap();
    fs::remove_file(&path).unwrap();
    assert_signature_verifies(&headers, TEST_PATH);
}

#[test]
fn sign_oci_request_rejects_a_non_rsa_key() {
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &signing_credentials(TEST_EC_KEY),
        &Json::Object(Object::new()),
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert_eq!(
        error.message,
        "The provided private key is not an RSA key, which is required for OCI signing."
    );
}

#[test]
fn sign_oci_request_rejects_a_malformed_key() {
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &signing_credentials("not a PEM key"),
        &Json::Object(Object::new()),
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(!error.message.is_empty());
}

#[test]
fn sign_oci_request_rejects_a_small_key() {
    let error = sign_oci_request(
        &mut HeaderMap::new(),
        &signing_credentials(TEST_SMALL_KEY),
        &Json::Object(Object::new()),
        TEST_API_BASE,
        &fixed_clock(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
}

#[test]
fn oci_error_converts_into_the_shared_provider_error() {
    let error = OciError {
        status: 401,
        message: "missing credentials".to_owned(),
    };
    let provider: ProviderError = ProviderError::from(error);
    assert_eq!(provider.status_code, 401);
    assert_eq!(provider.message, "missing credentials");
}

#[test]
fn oci_serving_mode_serializes_the_wire_shape() {
    let on_demand = OciServingMode::OnDemand {
        model_id: "cohere.command-r-plus".to_owned(),
    };
    let on_demand_json = serde_json::to_string(&on_demand).unwrap();
    assert_eq!(
        parse_json(&on_demand_json),
        parse_json(r#"{"servingType": "ON_DEMAND", "modelId": "cohere.command-r-plus"}"#)
    );
    let dedicated = OciServingMode::Dedicated {
        endpoint_id: "ocid1.endpoint.oc1..aaaa".to_owned(),
    };
    let dedicated_json = serde_json::to_string(&dedicated).unwrap();
    assert_eq!(
        parse_json(&dedicated_json),
        parse_json(r#"{"servingType": "DEDICATED", "endpointId": "ocid1.endpoint.oc1..aaaa"}"#)
    );
}

#[rstest]
#[case::inlines_nested_definitions(
    r##"{"$defs": {"Inner": {"type": "object", "properties": {"x": {"type": "string"}}}}, "a": {"$ref": "#/$defs/Inner"}}"##,
    r#"{"a": {"type": "object", "properties": {"x": {"type": "string"}}}}"#
)]
#[case::breaks_cycles(
    r##"{"$defs": {"Loop": {"$ref": "#/$defs/Loop"}}, "a": {"$ref": "#/$defs/Loop"}}"##,
    r#"{"a": {"type": "object"}}"#
)]
#[case::missing_definition_becomes_an_object(
    r##"{"$defs": {}, "a": {"$ref": "#/$defs/Nope"}}"##,
    r#"{"a": {"type": "object"}}"#
)]
#[case::missing_defs_table_becomes_an_object(
    r##"{"$defs": "no", "a": {"$ref": "#/$defs/Nope"}}"##,
    r#"{"a": {"type": "object"}}"#
)]
#[case::external_ref_stays_put(
    r#"{"a": {"$ref": "https://example.com/schema.json", "description": "d"}}"#,
    r#"{"a": {"$ref": "https://example.com/schema.json", "description": "d"}}"#
)]
#[case::non_string_ref_stays_put(r#"{"a": {"$ref": 7}}"#, r#"{"a": {"$ref": 7}}"#)]
#[case::null_ref_is_recursed(
    r##"{"$defs": {"A": {"type": "string"}}, "a": {"$ref": null}, "b": {"$ref": "#/$defs/A"}}"##,
    r#"{"a": {"$ref": null}, "b": {"type": "string"}}"#
)]
#[case::last_segment_names_the_definition(
    r##"{"$defs": {"A": {"properties": {"B": {"type": "string"}}}}, "x": {"$ref": "#/$defs/A/B"}}"##,
    r#"{"x": {"type": "object"}}"#
)]
fn resolve_oci_schema_refs_matches_litellm(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        resolve_oci_schema_refs(&parse_json(input)).unwrap(),
        parse_json(expected)
    );
}

#[test]
fn resolve_oci_schema_refs_bounds_a_long_ref_chain() {
    let chain = (0_u32..20_000_u32)
        .zip(1_u32..=20_000_u32)
        .map(|(from, to)| format!("\"c{from}\": {{\"$ref\": \"#/$defs/c{to}\"}}"))
        .collect::<Vec<String>>()
        .join(", ");
    let schema = format!("{{\"$defs\": {{{chain}}}, \"a\": {{\"$ref\": \"#/$defs/c0\"}}}}");
    let resolved = resolve_oci_schema_refs(&parse_json(&schema)).unwrap();
    assert_eq!(resolved, parse_json(r#"{"a": {"type": "object"}}"#));
}

#[test]
fn resolve_oci_schema_refs_bounds_a_nested_ref_chain() {
    let definitions = (0_u32..33_u32)
        .zip(1_u32..=33_u32)
        .map(|(index, next)| {
            let base = format!(r##"{{"$ref": "#/$defs/c{next}"}}"##);
            let layered =
                (0_u32..100_u32).fold(base, |value, _| format!(r#"{{"properties": {value}}}"#));
            format!("\"c{index}\": {layered}")
        })
        .collect::<Vec<String>>()
        .join(", ");
    let schema = format!("{{\"$defs\": {{{definitions}}}, \"a\": {{\"$ref\": \"#/$defs/c0\"}}}}");
    let worker = thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || resolve_oci_schema_refs(&parse_json(&schema)))
        .unwrap();
    let resolved = worker.join().unwrap().unwrap();
    let entry = resolved
        .as_object()
        .and_then(|object| object.get("a"))
        .unwrap();
    assert_eq!(deepest_value(entry), &parse_json(r#"{"type": "object"}"#));
}

#[test]
fn resolve_oci_schema_refs_rejects_an_exponential_expansion() {
    let definitions = (0_u32..30_u32)
        .zip(1_u32..=30_u32)
        .map(|(index, next)| {
            format!(
                "\"c{index}\": {{\"allOf\": [{{\"$ref\": \"#/$defs/c{next}\"}}, {{\"$ref\": \"#/$defs/c{next}\"}}]}}"
            )
        })
        .collect::<Vec<String>>()
        .join(", ");
    let schema = format!("{{\"$defs\": {{{definitions}}}, \"a\": {{\"$ref\": \"#/$defs/c0\"}}}}");
    let error = resolve_oci_schema_refs(&parse_json(&schema)).unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("expands past"));
}

#[rstest]
#[case::first_non_null_branch_merged(
    r#"{"anyOf": [{"type": "null"}, {"type": "string", "maxLength": 3}], "description": "d"}"#,
    r#"{"description": "d", "type": "string", "maxLength": 3}"#
)]
#[case::branch_keys_win(
    r#"{"anyOf": [{"type": "object", "properties": {"z": {"type": "number"}}}], "title": "O"}"#,
    r#"{"title": "O", "type": "object", "properties": {"z": {"type": "number"}}}"#
)]
#[case::objects_with_a_type_are_skipped(
    r#"{"type": "string", "anyOf": [{"type": "null"}, {"type": "integer"}]}"#,
    r#"{"type": "string", "anyOf": [{"type": "null"}, {"type": "integer"}]}"#
)]
#[case::without_a_non_null_branch_it_recurses(
    r#"{"anyOf": [{"type": "null"}], "z": {"anyOf": [{"type": "null"}, {"type": "boolean"}]}}"#,
    r#"{"anyOf": [{"type": "null"}], "z": {"type": "boolean"}}"#
)]
#[case::a_non_list_any_of_recurses(r#"{"anyOf": "nope"}"#, r#"{"anyOf": "nope"}"#)]
#[case::a_non_object_first_branch_merges_nothing(r#"{"anyOf": ["x", {"type": "null"}]}"#, "{}")]
fn resolve_oci_schema_anyof_matches_litellm(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        resolve_oci_schema_anyof(&parse_json(input)),
        parse_json(expected)
    );
}

#[rstest]
#[case::drops_title_and_null_default(
    r#"{"title": "T", "default": null, "type": "string"}"#,
    r#"{"type": "string"}"#
)]
#[case::keeps_a_non_null_default(r#"{"default": 0}"#, r#"{"default": 0}"#)]
#[case::type_any_becomes_object(r#"{"type": "any"}"#, r#"{"type": "object"}"#)]
#[case::list_type_takes_the_first_non_null(
    r#"{"type": ["null", "integer"]}"#,
    r#"{"type": "integer"}"#
)]
#[case::list_type_of_only_null_becomes_string(r#"{"type": ["null"]}"#, r#"{"type": "string"}"#)]
#[case::list_type_keeps_a_non_string_entry(r#"{"type": ["null", 3, "integer"]}"#, r#"{"type": 3}"#)]
#[case::array_without_items_gets_object_items(
    r#"{"type": "array"}"#,
    r#"{"type": "array", "items": {"type": "object"}}"#
)]
#[case::array_with_items_keeps_them(
    r#"{"type": "array", "items": null}"#,
    r#"{"type": "array", "items": null}"#
)]
#[case::required_is_filtered_to_properties(
    r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a", "zz", 3]}"#,
    r#"{"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]}"#
)]
#[case::required_without_properties_is_kept(r#"{"required": ["a"]}"#, r#"{"required": ["a"]}"#)]
#[case::non_list_required_becomes_empty(r#"{"required": null}"#, r#"{"required": []}"#)]
#[case::recurses_into_properties(
    r#"{"properties": {"a": {"title": "T", "default": null, "type": "any"}}}"#,
    r#"{"properties": {"a": {"type": "object"}}}"#
)]
#[case::keeps_a_property_named_title(
    r#"{"type": "object", "properties": {"title": {"type": "string"}, "x": {"type": "number", "title": "X"}}, "required": ["title", "x"]}"#,
    r#"{"type": "object", "properties": {"title": {"type": "string"}, "x": {"type": "number"}}, "required": ["title", "x"]}"#
)]
#[case::keeps_a_property_named_required(
    r#"{"properties": {"required": {"type": "boolean"}, "title": {"type": "string"}}, "required": ["required", "title"]}"#,
    r#"{"properties": {"required": {"type": "boolean"}, "title": {"type": "string"}}, "required": ["required", "title"]}"#
)]
#[case::keeps_a_property_named_default(
    r#"{"properties": {"default": null, "x": {"type": "string"}}}"#,
    r#"{"properties": {"default": null, "x": {"type": "string"}}}"#
)]
#[case::recurses_into_lists(r#"[{"title": "T"}]"#, "[{}]")]
fn sanitize_oci_schema_matches_litellm(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        sanitize_oci_schema(&parse_json(input)),
        parse_json(expected)
    );
}

#[test]
fn oci_json_to_python_types_maps_every_json_type() {
    assert_eq!(
        OCI_JSON_TO_PYTHON_TYPES,
        [
            ("string", "str"),
            ("number", "float"),
            ("boolean", "bool"),
            ("integer", "int"),
            ("array", "List"),
            ("object", "Dict"),
            ("any", "any"),
        ]
    );
}

#[test]
fn enrich_cohere_description_appends_constraints() {
    let schema = parse_json(
        r#"{"enum": ["a", "b", 3, true, null], "format": "date-time", "minimum": 1, "maximum": 5.5, "pattern": "^x$"}"#,
    );
    assert_eq!(
        enrich_cohere_description("Start", schema.as_object().unwrap()),
        "Start. Allowed values: ['a', 'b', 3, True, None]. Format: date-time. Range: min=1, max=5.5. Pattern: ^x$"
    );
}

#[test]
fn enrich_cohere_description_quotes_enum_values_like_python() {
    let schema = parse_json(r#"{"enum": ["a'b", "c\\d"]}"#);
    assert_eq!(
        enrich_cohere_description("", schema.as_object().unwrap()),
        "Allowed values: [\"a'b\", 'c\\\\d']"
    );
}

#[test]
fn enrich_cohere_description_keeps_each_bound_on_its_own() {
    let minimum = parse_json(r#"{"minimum": 0}"#);
    assert_eq!(
        enrich_cohere_description("d", minimum.as_object().unwrap()),
        "d. Range: min=0"
    );
    let maximum = parse_json(r#"{"maximum": 10}"#);
    assert_eq!(
        enrich_cohere_description("d", maximum.as_object().unwrap()),
        "d. Range: max=10"
    );
    let absent = parse_json("{}");
    assert_eq!(
        enrich_cohere_description("d", absent.as_object().unwrap()),
        "d"
    );
    assert_eq!(
        enrich_cohere_description("", absent.as_object().unwrap()),
        ""
    );
}
