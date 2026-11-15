use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::target::RegionInputs;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::Builder;

use super::support::params;
use crate::s3_vectors_search::config::{RegionEnvironment, complete_url};
use crate::s3_vectors_search::request::{QueryInput, QueryRequest, build_query_request};
use crate::s3_vectors_search::signing::{SignSearch, sign_search_request};

fn header<'signed>(signed: &'signed AwsSignedRequest, name: &str) -> &'signed str {
    signed
        .headers
        .iter()
        .find(|(found, _)| found.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .unwrap()
}

fn query_request() -> QueryRequest {
    let litellm_params = params("{}");
    let api_base = complete_url(&litellm_params, RegionEnvironment::default()).unwrap();
    build_query_request(&QueryInput {
        vector_store_id: "docs:index-1",
        query_text: "hello",
        query_vector: &[0.5_f64, 1.0_f64],
        max_num_results: None,
        api_base: &api_base,
        litellm_params: &litellm_params,
    })
    .unwrap()
}

fn region_optional_params(region: Option<&str>) -> BTreeMap<String, Box<RawValue>> {
    region
        .map(|name| {
            (
                "aws_region_name".to_owned(),
                RawValue::from_string(format!("\"{name}\"")).unwrap(),
            )
        })
        .into_iter()
        .collect()
}

fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("AKIAFIXTUREACCESS0001"),
        secret_access_key: SecretString::from("fixture-secret-access-key"),
        session_token: None,
    }
}

fn sign(region: Option<&str>, api_key: Option<&SecretString>) -> AwsSignedRequest {
    let query = query_request();
    let optional_params = region_optional_params(region);
    let headers = [(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let now = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime
        .block_on(sign_search_request(
            SignSearch {
                headers: &headers,
                optional_params: &optional_params,
                request_data: &query.body,
                api_base: &query.url,
                api_key,
                configured_bearer: None,
                region_inputs: RegionInputs::default(),
                now,
            },
            |_settings, _region| ready(Ok::<Credentials, _>(credentials())),
        ))
        .unwrap()
}

#[test]
fn the_query_is_signed_for_service_s3vectors() {
    let signed = sign(Some("eu-west-1"), None);
    assert_eq!(signed.method, "POST");
    assert_eq!(
        signed.url,
        "https://s3vectors.us-west-2.api.aws/QueryVectors"
    );
    let authorization = header(&signed, "authorization");
    assert!(
        authorization
            .contains("Credential=AKIAFIXTUREACCESS0001/20231114/eu-west-1/s3vectors/aws4_request"),
        "{authorization}"
    );
    assert_eq!(header(&signed, "x-amz-date"), "20231114T221320Z");
    assert_eq!(header(&signed, "content-type"), "application/json");
}

#[test]
fn the_signed_body_is_the_python_json_of_the_query() {
    let signed = sign(None, None);
    assert_eq!(
        String::from_utf8(signed.body).unwrap(),
        r#"{"vectorBucketName": "docs", "indexName": "index-1", "queryVector": {"float32": [0.5, 1.0]}, "topK": 5, "returnDistance": true, "returnMetadata": true}"#
    );
}

#[test]
fn the_signing_region_defaults_when_no_region_is_given() {
    let authorization = sign(None, None);
    assert!(header(&authorization, "authorization").contains("/us-west-2/s3vectors/aws4_request"));
}

#[test]
fn a_bearer_api_key_replaces_the_signature() {
    let key = SecretString::from("fixture-bearer-key");
    let signed = sign(None, Some(&key));
    assert_eq!(
        header(&signed, "authorization"),
        "Bearer fixture-bearer-key"
    );
}
