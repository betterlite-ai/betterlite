#![cfg(test)]

use std::fmt::Debug;
use std::task::{Context, Poll, Waker};

use futures_util::{StreamExt as _, stream};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::aws_params::{map_auth_aliases, merge_aws_request_params};
use super::validation::validate_request_metadata;
use super::{
    BedrockRequestPolicy, OneResponse, Params, REQUEST_METADATA_HEADER, RequestValue,
    merge_invoke_headers,
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

fn entries(pairs: &[(&str, RequestValue)]) -> Params {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn mapping(pairs: &[(&str, RequestValue)]) -> RequestValue {
    RequestValue::Mapping(entries(pairs))
}

fn metadata(pairs: &[(&str, RequestValue)]) -> Params {
    entries(&[("metadata", mapping(pairs))])
}

fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn policy(fields: &[&str]) -> Option<BedrockRequestPolicy> {
    let owned: Vec<String> = fields.iter().map(|field| (*field).to_owned()).collect();
    BedrockRequestPolicy::from_fields(&owned)
}

// IndexMap equality ignores order, and order is part of every outcome here
fn rendered<Value>(value: &Value) -> String
where
    Value: Debug,
{
    format!("{value:?}")
}

fn merged(
    original: &IndexMap<String, String>,
    derived: &IndexMap<String, String>,
    owner: Option<&BedrockRequestPolicy>,
    params: &Params,
) -> String {
    let guardrail: Vec<(String, String)> = derived.clone().into_iter().collect();
    rendered(&merge_invoke_headers(original, &guardrail, owner, params).unwrap())
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
    let outcomes = owners.map(|owner| merged(&spoofed, &headers(&[]), owner.as_ref(), &params));
    let empty = rendered(&headers(&[]));
    assert_eq!(outcomes, [empty.clone(), empty, rendered(&spoofed)]);
}

fn request_metadata_size_characters() {
    let long = "k".repeat(257);
    let longest = "k".repeat(256);
    let many: Params = (0_u8..17)
        .map(|index| (format!("k{index}"), text("v")))
        .collect();
    let inputs = [
        text("a"),
        RequestValue::Mapping(many.clone()),
        RequestValue::Mapping(many.into_iter().take(16).collect()),
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
    assert_eq!(rendered(&outcomes), rendered(&expected));
    errors_are_bad_requests(&outcomes);
    resolver_drops_invalid_pairs(&long);
}

fn errors_are_bad_requests(outcomes: &[Result<(), BedrockError>]) {
    let statuses: Vec<u16> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().err())
        .map(BedrockError::status_code)
        .collect();
    assert_eq!(statuses, [400; 8]);
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
    let resolved = policy(&["x"]).and_then(|owner| owner.resolve(&Params::new(), Some(&caller)));
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
    let none = Params::new();
    let unchanged = merged(&original, &headers(&[]), None, &none);
    assert_eq!(unchanged, rendered(&original));
    let guarded = merged(&original, &guardrail, None, &none);
    assert_eq!(guarded, rendered(&headers(&filled)));
    let owner = policy(&["user_api_key_hash"]);
    let params = metadata(&[("user_api_key_hash", text("h"))]);
    let json = "{\"user_api_key_hash\":\"h\"}";
    let mut spoofed = headers(&[("x-amzn-bedrock-request-metadata", "a"), ("Accept", "json")]);
    let proxy_named = headers(&[(REQUEST_METADATA_HEADER, "g"), ("X-Extra", "e")]);
    let evicted = merged(&spoofed, &proxy_named, owner.as_ref(), &params);
    spoofed.shift_remove_index(0);
    let overridden = merged(&spoofed, &proxy_named, owner.as_ref(), &params);
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
    assert_eq!(evicted, rendered(&headers(&evicted_expected)));
    assert_eq!(overridden, rendered(&headers(&overridden_expected)));
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
    let mut empty = Params::new();
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
    let owner = policy(&["user_api_key_alias", "user_api_key_hash"]).unwrap();
    let spoofed = headers(&[
        (REQUEST_METADATA_HEADER, "spoof"),
        ("x-amzn-bedroc\u{212a}-request-metadata", "kelvin"),
        ("Host", "h"),
    ]);
    assert_eq!(owner.metadata_header(&Params::new()).unwrap(), None);
    let emptied = merged(&spoofed, &headers(&[]), Some(&owner), &Params::new());
    assert_eq!(emptied, rendered(&headers(&[("Host", "h")])));
    let hash = text("h\u{3000}1\t\u{b}\u{85}");
    let params = metadata(&[
        ("user_api_key_hash", hash),
        ("user_api_key_alias", text("team/a")),
    ]);
    let json =
        "{\"user_api_key_alias\":\"team/a\",\"user_api_key_hash\":\"h\\u30001\\t\\u000b\\u0085\"}";
    let header = owner.metadata_header(&params).unwrap();
    assert_eq!(header.as_deref(), Some(json));
    let replaced = merged(&spoofed, &headers(&[]), Some(&owner), &params);
    let expected = headers(&[("Host", "h"), (REQUEST_METADATA_HEADER, json)]);
    assert_eq!(replaced, rendered(&expected));
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
    let resolved = owner.resolve(&params, Some(&caller));
    assert_eq!(rendered(&resolved), rendered(&Some(headers(&expected))));
    budget_caps(&owner, &params, &caller);
}

fn identity_params() -> Params {
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

fn budget_caps(owner: &BedrockRequestPolicy, params: &Params, caller: &RequestValue) {
    let crowded = RequestValue::Mapping(
        (0_u8..20)
            .map(|index| (format!("k{index:02}"), text("v")))
            .collect(),
    );
    let budgeted = owner.resolve(params, Some(&crowded)).unwrap();
    let key = |index: usize| budgeted.get_index(index).map(|(name, _)| name.as_str());
    assert_eq!(
        (budgeted.len(), key(2), key(15)),
        (16, Some("k00"), Some("k13"))
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
        .resolve(
            &Params::from([("metadata".to_owned(), identities)]),
            Some(caller),
        )
        .unwrap();
    let last = capped.get_index(15).map(|(name, _)| name.as_str());
    assert_eq!((capped.len(), last), (16, Some("user_api_key_f15")));
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
    kept.insert("aws_region_name".to_owned(), text(""));
    kept.insert("aws_access_key_id".to_owned(), text("AKIA_DEPLOY"));
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
    let access_key = |params: &Params| match params.first() {
        Some((_, RequestValue::Secret(key))) => Some(key.expose_secret().to_owned()),
        _ => None,
    };
    let expected_keys = ["AKIA_FIRST", "AKIA_SECOND"].map(|key| Some(key.to_owned()));
    assert_eq!([access_key(&first), access_key(&second)], expected_keys);
    assert!(
        !first
            .iter()
            .chain(&second)
            .any(|(name, _)| name == "aws_profile_name")
    );
    let owner = policy(&["user_api_key_hash"]).unwrap();
    let header = |hash: &str| {
        let params = metadata(&[("user_api_key_hash", text(hash))]);
        owner.metadata_header(&params).unwrap()
    };
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
    let merged = rendered(&merge_aws_request_params(&deployment, &Params::new()));
    assert!(merged.contains("aws_secret_access_key"));
    assert!(!merged.contains("fixture-secret-value") && !merged.contains("AKIA_FIXTURE"));
    let invalid = mapping(&[("token", text("fixture-secret-value!"))]);
    let error = validate_request_metadata(&invalid).unwrap_err();
    assert!(matches!(&error, BedrockError::MetadataValueCharacters { key } if key == "token"));
    assert!(!format!("{error} {error:?}").contains("fixture-secret-value"));
}
