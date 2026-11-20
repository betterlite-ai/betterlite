#![cfg(test)]

use super::{
    CREATE_ROLE, CallerIdentity, PUT_ROLE_POLICY, caller_identity, caller_identity_from,
    child_text, iam_request, sts_url,
};
use crate::bedrock_provisioning::calls::CallRequest;
use crate::bedrock_provisioning::plan::{self, RoleResources};

fn resources<'plan>(arn_prefix: &'plan str, region: &'plan str) -> RoleResources<'plan> {
    RoleResources {
        arn_prefix,
        region,
        account_id: "123456789012",
        embedding_model: "amazon.titan-embed-text-v2:0",
        collection_arn: "arn:aws:aoss:us-west-2:123456789012:collection/abc",
        bucket: "litellm-kb-0123abcd",
    }
}

fn header(call: &CallRequest, name: &str) -> Option<String> {
    call.headers
        .iter()
        .find(|(present, _)| present.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

#[test]
fn call_caller_identity_posts_the_sts_form() {
    let call = caller_identity("us-west-2");
    assert_eq!(call.method, "POST");
    assert_eq!(call.service, "sts");
    assert_eq!(call.signing_region, "us-west-2");
    assert_eq!(call.url, "https://sts.us-west-2.amazonaws.com/");
    assert_eq!(
        call.body,
        b"Action=GetCallerIdentity&Version=2011-06-15".to_vec()
    );
    assert_eq!(
        header(&call, "Content-Type").as_deref(),
        Some("application/x-www-form-urlencoded; charset=utf-8")
    );
}

#[test]
fn call_sts_url_follows_the_partition_suffix() {
    assert_eq!(
        sts_url("cn-north-1"),
        "https://sts.cn-north-1.amazonaws.com.cn/"
    );
    assert_eq!(
        sts_url("us-gov-west-1"),
        "https://sts.us-gov-west-1.amazonaws.com/"
    );
}

#[test]
fn call_caller_identity_reads_account_and_arn() {
    let body = br#"<GetCallerIdentityResponse><GetCallerIdentityResult><Arn>arn:aws:iam::123456789012:role/kb</Arn><UserId>AIDA</UserId><Account>123456789012</Account></GetCallerIdentityResult></GetCallerIdentityResponse>"#;
    let identity = caller_identity_from(body).unwrap();
    assert_eq!(
        identity,
        CallerIdentity {
            account: "123456789012".to_owned(),
            arn: "arn:aws:iam::123456789012:role/kb".to_owned(),
        }
    );
}

#[test]
fn call_caller_identity_reports_a_missing_element() {
    let body =
        br#"<GetCallerIdentityResult><Account>123456789012</Account></GetCallerIdentityResult>"#;
    assert!(caller_identity_from(body).is_err());
}

#[test]
fn call_caller_identity_rejects_a_body_that_is_not_utf8() {
    assert!(caller_identity_from(&[0xff, 0xfe]).is_err());
}

#[test]
fn call_create_role_posts_the_encoded_policy() {
    let policy = plan::trust_policy(&resources("arn:aws:", "us-west-2"));
    let document = serde_json::to_string(&policy).unwrap();
    let call = iam_request(
        "us-west-2",
        CREATE_ROLE,
        &[
            ("RoleName", "litellm-bedrock-kb-0123abcd"),
            ("AssumeRolePolicyDocument", document.as_str()),
        ],
    );
    assert_eq!(call.operation, "CreateRole");
    assert_eq!(call.service, "iam");
    assert_eq!(call.signing_region, "us-east-1");
    assert_eq!(call.url, "https://iam.amazonaws.com/");
    let body = std::str::from_utf8(&call.body).unwrap();
    assert!(body.starts_with("Action=CreateRole&Version=2010-05-08"));
    assert!(body.contains("&RoleName=litellm-bedrock-kb-0123abcd&"));
    assert!(body.contains("&AssumeRolePolicyDocument=%7B%22Version%22%3A%222012-10-17%22"));
    assert!(!body.contains('{'));
}

#[test]
fn call_iam_endpoints_are_global_per_partition() {
    let commercial = iam_request("us-west-2", CREATE_ROLE, &[]);
    assert_eq!(commercial.url, "https://iam.amazonaws.com/");
    assert_eq!(commercial.signing_region, "us-east-1");
    let china = iam_request("cn-north-1", CREATE_ROLE, &[]);
    assert_eq!(china.url, "https://iam.cn-north-1.amazonaws.com.cn/");
    assert_eq!(china.signing_region, "cn-north-1");
    let government = iam_request("us-gov-west-1", CREATE_ROLE, &[]);
    assert_eq!(government.url, "https://iam.us-gov.amazonaws.com/");
    assert_eq!(government.signing_region, "us-gov-west-1");
}

#[test]
fn call_create_role_reads_the_role_arn() {
    let body = br#"<CreateRoleResponse><CreateRoleResult><Role><Path>/</Path><Arn>arn:aws:iam::123456789012:role/litellm-bedrock-kb-0123abcd</Arn></Role></CreateRoleResult></CreateRoleResponse>"#;
    assert_eq!(
        child_text(body, "Role", "Arn", CREATE_ROLE).unwrap(),
        "arn:aws:iam::123456789012:role/litellm-bedrock-kb-0123abcd"
    );
}

#[test]
fn call_role_arn_reports_a_missing_element() {
    let body = br#"<CreateRoleResponse><CreateRoleResult><Role><Path>/</Path></Role></CreateRoleResult></CreateRoleResponse>"#;
    assert!(child_text(body, "Role", "Arn", CREATE_ROLE).is_err());
}

#[test]
fn call_put_role_policy_sends_the_policy_name_and_document() {
    let policy = plan::permissions_policy(&resources("arn:aws:", "us-west-2"));
    let document = serde_json::to_string(&policy).unwrap();
    let call = iam_request(
        "us-west-2",
        PUT_ROLE_POLICY,
        &[
            ("RoleName", "role"),
            ("PolicyName", "role-policy"),
            ("PolicyDocument", document.as_str()),
        ],
    );
    assert_eq!(call.operation, "PutRolePolicy");
    let body = std::str::from_utf8(&call.body).unwrap();
    assert!(body.starts_with("Action=PutRolePolicy&Version=2010-05-08&RoleName=role"));
    assert!(body.contains("&PolicyName=role-policy&"));
    assert!(body.contains("PolicyDocument=%7B"));
}

#[test]
fn call_form_values_encode_reserved_characters() {
    let policy = plan::permissions_policy(&resources("arn:aws:", "us-west-2"));
    let document = serde_json::to_string(&policy).unwrap();
    let call = iam_request(
        "us-west-2",
        PUT_ROLE_POLICY,
        &[("PolicyDocument", document.as_str())],
    );
    let body = std::str::from_utf8(&call.body).unwrap();
    assert!(body.contains("aoss%3AAPIAccessAll"));
    assert!(body.contains("%2F"));
    assert!(body.contains("%2A"));
    assert!(!body.contains("arn:aws:aoss:us-west-2:123456789012:collection/abc"));
}

#[test]
fn call_xml_entities_are_decoded() {
    let body = br#"<Role><Arn>arn:aws:iam::1:role/a&amp;b</Arn></Role>"#;
    assert_eq!(
        child_text(body, "Role", "Arn", CREATE_ROLE).unwrap(),
        "arn:aws:iam::1:role/a&b"
    );
}

#[test]
fn call_xml_cdata_is_read() {
    let body = br#"<Role><Arn><![CDATA[arn:aws:iam::1:role/c]]></Arn></Role>"#;
    assert_eq!(
        child_text(body, "Role", "Arn", CREATE_ROLE).unwrap(),
        "arn:aws:iam::1:role/c"
    );
}

#[test]
fn call_xml_without_the_parent_element_is_missing() {
    let body = br#"<Role><Arn>arn:aws:iam::1:role/a</Arn></Role>"#;
    assert!(caller_identity_from(body).is_err());
}
