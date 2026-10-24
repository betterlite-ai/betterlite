#![cfg(test)]

use std::task::{Context, Poll, Waker};

use futures_util::{StreamExt as _, stream};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::aws_params::{map_auth_aliases, merge_aws_request_params};
use super::validation::validate_request_metadata;
use super::{
    BedrockRequestPolicy, OneResponse, REQUEST_METADATA_HEADER, RequestValue, merge_invoke_headers,
};
use crate::error::BedrockError;

fn text(value: &str) -> RequestValue {
    RequestValue::Text(value.to_owned())
}

fn raw(value: &str) -> RequestValue {
    RequestValue::Raw(value.to_owned())
}

fn secret(value: &str) -> RequestValue {
    RequestValue::Secret(SecretString::from(value))
}

fn entries(pairs: &[(&str, RequestValue)]) -> Vec<(String, RequestValue)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn mapping(pairs: &[(&str, RequestValue)]) -> RequestValue {
    RequestValue::Mapping(entries(pairs))
}

fn metadata(pairs: &[(&str, RequestValue)]) -> Vec<(String, RequestValue)> {
    entries(&[("metadata", mapping(pairs))])
}

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn policy(fields: &[&str]) -> Option<BedrockRequestPolicy> {
    let owned: Vec<String> = fields.iter().map(|field| (*field).to_owned()).collect();
    BedrockRequestPolicy::from_fields(&owned)
}

fn rendered(params: &[(String, RequestValue)]) -> String {
    format!("{params:?}")
}

#[rstest]
#[case::request_metadata_owner(request_metadata_owner)]
#[case::request_metadata_size_characters(request_metadata_size_characters)]
#[case::unowned_user_metadata(unowned_user_metadata)]
#[case::auth_alias_mapping(auth_alias_mapping)]
#[case::one_response_iterator(one_response_iterator)]
#[case::owned_empty_removes_spoofed_header(owned_empty_removes_spoofed_header)]
#[case::identity_before_client_budget(identity_before_client_budget)]
#[case::deployment_auth_merge(deployment_auth_merge)]
fn request_policy_cases(#[case] check: fn()) {
    check();
}

fn request_metadata_owner() {
    let spend = mapping(&[("team", text("a"))]);
    let params = metadata(&[("nonsense", text("x")), ("spend_logs_metadata", spend)]);
    let spoofed = headers(&[("x-amzn-bedrock-request-metadata", "spoof")]);
    let repeated = policy(&[
        "user_api_key_hash",
        "spend_logs_metadata",
        "user_api_key_hash",
        "",
    ]);
    let distinct = policy(&["user_api_key_hash", "spend_logs_metadata", ""]);
    assert_eq!((policy(&[]), repeated), (None, distinct));
    assert_eq!(policy(&["nonsense"]).unwrap().resolve(&params, None), None);
    let owners = [policy(&["nonsense"]), policy(&[""]), None];
    let merged = owners.map(|owner| merge_invoke_headers(&spoofed, &[], owner.as_ref(), &params));
    assert_eq!(merged, [Vec::new(), Vec::new(), spoofed]);
}

fn request_metadata_size_characters() {
    let long = "k".repeat(257);
    let longest = "k".repeat(256);
    let many: Vec<(String, RequestValue)> = (0_u8..17)
        .map(|index| (format!("k{index}"), text("v")))
        .collect();
    let inputs = [
        text("a"),
        RequestValue::Mapping(many.clone()),
        RequestValue::Mapping(many[..16].to_vec()),
        mapping(&[("ok", text("v")), ("", raw("1"))]),
        mapping(&[("", text("v"))]),
        mapping(&[(&long, text("v"))]),
        mapping(&[("k", text(&long))]),
        mapping(&[(&longest, text(&longest)), ("empty", text(""))]),
        mapping(&[(
            "a\u{3000}b\u{1c}",
            text(" \t\n\u{b}\u{a0}\u{85}\u{2028}\u{1f}:_@$#=/+,.-"),
        )]),
        mapping(&[("bad!", text("v"))]),
        mapping(&[("k", text("zero\u{200b}width"))]),
    ];
    let key = |name: &str| name.to_owned();
    let outcomes: Vec<Result<(), BedrockError>> =
        inputs.iter().map(validate_request_metadata).collect();
    let expected = [
        Err(BedrockError::MetadataNotObject),
        Err(BedrockError::TooManyMetadataPairs { count: 17 }),
        Ok(()),
        Err(BedrockError::MetadataValueNotText { key: key("") }),
        Err(BedrockError::MetadataKeyLength { length: 0 }),
        Err(BedrockError::MetadataKeyLength { length: 257 }),
        Err(BedrockError::MetadataValueLength { key: key("k") }),
        Ok(()),
        Ok(()),
        Err(BedrockError::MetadataKeyCharacters { key: key("bad!") }),
        Err(BedrockError::MetadataValueCharacters { key: key("k") }),
    ];
    assert_eq!(outcomes, expected);
    resolver_drops_invalid_pairs(&long);
}

fn resolver_drops_invalid_pairs(long: &str) {
    let caller = mapping(&[
        ("ok", text("v")),
        ("bad!", text("v")),
        ("k", text("\u{e9}t\u{e9}")),
        ("n", raw("1")),
        (long, text("v")),
        ("long", text(long)),
    ]);
    let resolved = policy(&["x"]).and_then(|owner| owner.resolve(&[], Some(&caller)));
    assert_eq!(resolved, Some(headers(&[("ok", "v")])));
}

fn unowned_user_metadata() {
    let original = headers(&[(REQUEST_METADATA_HEADER, "caller"), ("X-OTHER", "")]);
    let guardrail = headers(&[
        ("x-other", "g"),
        ("x-amzn-bedrock-guardrailIdentifier", "id"),
    ]);
    let filled = [
        (REQUEST_METADATA_HEADER, "caller"),
        ("X-OTHER", ""),
        ("x-amzn-bedrock-guardrailIdentifier", "id"),
    ];
    assert_eq!(merge_invoke_headers(&original, &[], None, &[]), original);
    assert_eq!(
        merge_invoke_headers(&original, &guardrail, None, &[]),
        headers(&filled)
    );
    let owner = policy(&["user_api_key_hash"]);
    let params = metadata(&[("user_api_key_hash", text("h"))]);
    let json = "{\"user_api_key_hash\":\"h\"}";
    let spoofed = headers(&[("x-amzn-bedrock-request-metadata", "a"), ("Accept", "json")]);
    let proxy_named = headers(&[(REQUEST_METADATA_HEADER, "g"), ("X-Extra", "e")]);
    let evicted = merge_invoke_headers(&spoofed, &proxy_named, owner.as_ref(), &params);
    let overridden = merge_invoke_headers(&spoofed[1..], &proxy_named, owner.as_ref(), &params);
    let evicted_expected = [
        ("Accept", "json"),
        ("X-Extra", "e"),
        (REQUEST_METADATA_HEADER, json),
    ];
    let overridden_expected = [
        ("Accept", "json"),
        (REQUEST_METADATA_HEADER, json),
        ("X-Extra", "e"),
    ];
    assert_eq!(evicted, headers(&evicted_expected));
    assert_eq!(overridden, headers(&overridden_expected));
}

fn auth_alias_mapping() {
    let mut optional = entries(&[("aws_region_name", text("us-east-1")), ("keep", raw("1"))]);
    let non_default = entries(&[
        ("aws_region_name", text("x")),
        ("region_name", text("eu-west-1")),
        ("vertex_location", text("y")),
    ]);
    map_auth_aliases(&non_default, &mut optional);
    let expected = entries(&[("aws_region_name", text("eu-west-1")), ("keep", raw("1"))]);
    assert_eq!(rendered(&optional), rendered(&expected));
    let mut empty = Vec::new();
    map_auth_aliases(&entries(&[("region_name", raw("null"))]), &mut empty);
    assert_eq!(
        rendered(&empty),
        rendered(&entries(&[("aws_region_name", raw("null"))]))
    );
}

fn one_response_iterator() {
    let mut context = Context::from_waker(Waker::noop());
    let mut absent: OneResponse<Option<&str>> = Some(None).into_iter();
    let yielded = absent.next();
    let streamed = stream::iter(&mut absent).poll_next_unpin(&mut context);
    assert_eq!(
        (yielded, streamed, absent.next()),
        (Some(None), Poll::Ready(None), None)
    );
    let mut response: OneResponse<&str> = Some("response").into_iter();
    let polled = stream::iter(&mut response).poll_next_unpin(&mut context);
    let drained = response.next();
    let exhausted = stream::iter(&mut response).poll_next_unpin(&mut context);
    let expected = (Poll::Ready(Some("response")), None, Poll::Ready(None));
    assert_eq!((polled, drained, exhausted), expected);
}

fn owned_empty_removes_spoofed_header() {
    let owner = policy(&["user_api_key_alias", "user_api_key_hash"]);
    let spoofed = headers(&[
        (REQUEST_METADATA_HEADER, "spoof"),
        ("x-amzn-bedroc\u{212a}-request-metadata", "kelvin"),
        ("Host", "h"),
    ]);
    assert_eq!(
        owner.as_ref().and_then(|owned| owned.metadata_header(&[])),
        None
    );
    let emptied = merge_invoke_headers(&spoofed, &[], owner.as_ref(), &[]);
    assert_eq!(emptied, headers(&[("Host", "h")]));
    let hash = text("h\u{3000}1\t\u{b}\u{85}");
    let params = metadata(&[
        ("user_api_key_hash", hash),
        ("user_api_key_alias", text("team/a")),
    ]);
    let json =
        "{\"user_api_key_alias\":\"team/a\",\"user_api_key_hash\":\"h\\u30001\\t\\u000b\\u0085\"}";
    let header = owner
        .as_ref()
        .and_then(|owned| owned.metadata_header(&params));
    assert_eq!(header.as_deref(), Some(json));
    let replaced = merge_invoke_headers(&spoofed, &[], owner.as_ref(), &params);
    assert_eq!(
        replaced,
        headers(&[("Host", "h"), (REQUEST_METADATA_HEADER, json)])
    );
}

fn identity_before_client_budget() {
    let owner = policy(&[
        "user_api_key_team_id",
        "user_api_key_hash",
        "user_api_key_alias",
        "spend_logs_metadata",
    ])
    .unwrap();
    let params = identity_params();
    let caller = mapping(&[
        ("z", text("1")),
        ("user_api_key_hash", text("spoof")),
        ("a", text("caller")),
        ("User_api_key_y", text("ok")),
    ]);
    let expected = [
        ("user_api_key_team_id", ""),
        ("user_api_key_alias", "alias"),
        ("z", "1"),
        ("a", "caller"),
        ("User_api_key_y", "ok"),
        ("c", "3"),
    ];
    assert_eq!(
        owner.resolve(&params, Some(&caller)),
        Some(headers(&expected))
    );
    budget_caps(&owner, &params, &caller);
}

fn identity_params() -> Vec<(String, RequestValue)> {
    let spend = mapping(&[
        ("a", text("spend")),
        ("user_api_key_x", text("spoof")),
        ("b", raw("2")),
        ("c", text("3")),
    ]);
    let first = mapping(&[
        ("user_api_key_hash", text("bad!")),
        ("user_api_key_alias", raw("1")),
    ]);
    let second = mapping(&[
        ("user_api_key_hash", text("good")),
        ("user_api_key_alias", text("alias")),
        ("user_api_key_team_id", text("")),
        ("spend_logs_metadata", spend),
    ]);
    entries(&[("metadata", first), ("litellm_metadata", second)])
}

fn budget_caps(
    owner: &BedrockRequestPolicy,
    params: &[(String, RequestValue)],
    caller: &RequestValue,
) {
    let crowded = RequestValue::Mapping(
        (0_u8..20)
            .map(|index| (format!("k{index:02}"), text("v")))
            .collect(),
    );
    let budgeted = owner.resolve(params, Some(&crowded)).unwrap();
    assert_eq!(
        (
            budgeted.len(),
            budgeted[2].0.as_str(),
            budgeted[15].0.as_str()
        ),
        (16, "k00", "k13")
    );
    let many: Vec<String> = (0_u8..17)
        .map(|index| format!("user_api_key_f{index:02}"))
        .collect();
    let identities = RequestValue::Mapping(
        many.iter()
            .map(|field| (field.clone(), text("v")))
            .collect(),
    );
    let capped = BedrockRequestPolicy::from_fields(&many)
        .unwrap()
        .resolve(&[("metadata".to_owned(), identities)], Some(caller))
        .unwrap();
    assert_eq!(
        (capped.len(), capped[15].0.as_str()),
        (16, "user_api_key_f15")
    );
    assert_eq!(
        owner.resolve(&entries(&[("metadata", text("user_api_key_hash"))]), None),
        None
    );
}

fn deployment_auth_merge() {
    let deployment = entries(&[
        ("aws_access_key_id", text("AKIA_DEPLOY")),
        ("aws_secret_access_key", text("deploy-secret")),
        ("aws_region_name", text("us-east-1")),
        ("aws_session_token", raw("null")),
        ("model", text("deployment-model")),
    ]);
    let request = entries(&[
        ("aws_profile_name", text("admin")),
        ("aws_session_token", text("request-token")),
        ("aws_region_name", text("eu-west-1")),
        ("aws_bedrock_runtime_endpoint", text("https://runtime")),
        ("model", text("request-model")),
        ("aws_role_name", text("role")),
        ("temperature", raw("0.2")),
    ]);
    let stripped = entries(&[
        ("aws_session_token", raw("null")),
        ("aws_region_name", text("us-east-1")),
        ("aws_bedrock_runtime_endpoint", text("https://runtime")),
        ("model", text("deployment-model")),
        ("temperature", raw("0.2")),
        ("aws_access_key_id", text("AKIA_DEPLOY")),
        ("aws_secret_access_key", text("deploy-secret")),
    ]);
    let merged = merge_aws_request_params(&deployment, &request);
    assert_eq!(rendered(&merged), rendered(&stripped));
    let incomplete = entries(&[
        ("aws_access_key_id", text("AKIA_DEPLOY")),
        ("aws_region_name", text("")),
    ]);
    let mut kept = request.clone();
    kept[2].1 = text("");
    kept.push(("aws_access_key_id".to_owned(), text("AKIA_DEPLOY")));
    assert_eq!(
        rendered(&merge_aws_request_params(&incomplete, &request)),
        rendered(&kept)
    );
}

#[test]
fn bedrock_request_policy_credential_identity_isolation() {
    let deployment = |key: &str| {
        let secret_key = format!("{key}-secret");
        entries(&[
            ("aws_access_key_id", secret(key)),
            ("aws_secret_access_key", secret(&secret_key)),
            ("aws_region_name", text("us-east-1")),
        ])
    };
    let request = entries(&[
        ("aws_access_key_id", text("AKIA_CALLER")),
        ("aws_profile_name", text("admin")),
    ]);
    let first = merge_aws_request_params(&deployment("AKIA_FIRST"), &request);
    let second = merge_aws_request_params(&deployment("AKIA_SECOND"), &request);
    let access_key = |params: &[(String, RequestValue)]| match params.first() {
        Some((_, RequestValue::Secret(key))) => Some(key.expose_secret().to_owned()),
        _ => None,
    };
    let keys = (access_key(&first), access_key(&second));
    assert_eq!(
        keys,
        (
            Some("AKIA_FIRST".to_owned()),
            Some("AKIA_SECOND".to_owned())
        )
    );
    assert!(
        !first
            .iter()
            .chain(&second)
            .any(|(name, _)| name == "aws_profile_name")
    );
    let owner = policy(&["user_api_key_hash"]).unwrap();
    let header =
        |hash: &str| owner.metadata_header(&metadata(&[("user_api_key_hash", text(hash))]));
    let expected = (
        Some(r#"{"user_api_key_hash":"first"}"#.to_owned()),
        Some(r#"{"user_api_key_hash":"second"}"#.to_owned()),
    );
    assert_eq!((header("first"), header("second")), expected);
}

#[test]
fn bedrock_request_policy_secret_safe_failure() {
    let deployment = entries(&[
        ("aws_access_key_id", secret("AKIA_FIXTURE")),
        ("aws_secret_access_key", secret("fixture-secret-value")),
        ("aws_region_name", text("us-east-1")),
    ]);
    let merged = rendered(&merge_aws_request_params(&deployment, &[]));
    assert!(merged.contains("aws_secret_access_key"));
    assert!(!merged.contains("fixture-secret-value") && !merged.contains("AKIA_FIXTURE"));
    let invalid = mapping(&[("token", text("fixture-secret-value!"))]);
    let error = validate_request_metadata(&invalid).unwrap_err();
    assert_eq!(
        error,
        BedrockError::MetadataValueCharacters {
            key: "token".to_owned()
        }
    );
    assert!(!format!("{error} {error:?}").contains("fixture-secret-value"));
}
