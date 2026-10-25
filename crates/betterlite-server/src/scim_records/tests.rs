#![cfg(test)]
use crate::scim_records::discovery::{ResourceType, Schema, ServiceProviderConfig};
use crate::scim_records::user::{Active, ScimBool, User};
use crate::scim_records::{ResourceKind, ScimResource, decode_scim_resource};
use rstest::rstest;
use std::fs;
use std::path::PathBuf;

fn dump(body: &str, kind: ResourceKind) -> String {
    serde_json::to_string(&decode_scim_resource(body.as_bytes(), kind).unwrap()).unwrap()
}
fn user(body: &str) -> User {
    let ScimResource::User(user) =
        decode_scim_resource(body.as_bytes(), ResourceKind::User).unwrap()
    else {
        panic!("not a user");
    };
    *user
}
#[rstest]
#[case::omitted(r#"{"schemas":[]}"#, Active::Defaulted(ScimBool(true)))]
#[case::explicit_true(r#"{"schemas":[],"active":true}"#, Active::Sent(ScimBool(true)))]
#[case::explicit_false(r#"{"schemas":[],"active":false}"#, Active::Sent(ScimBool(false)))]
#[case::lax_text(r#"{"schemas":[],"active":"False"}"#, Active::Sent(ScimBool(false)))]
#[case::lax_number(r#"{"schemas":[],"active":1}"#, Active::Sent(ScimBool(true)))]
fn active_keeps_whether_it_was_sent(#[case] body: &str, #[case] active: Active) {
    assert_eq!(user(body).active, active);
}
#[rstest]
#[case::null_active(r#"{"schemas":[],"active":null}"#, ResourceKind::User)]
#[case::number_two_active(r#"{"schemas":[],"active":2}"#, ResourceKind::User)]
#[case::user_without_schemas(r#"{"userName":"ada"}"#, ResourceKind::User)]
#[case::group_without_name(r#"{"schemas":[]}"#, ResourceKind::Group)]
#[case::padded_op(r#"{"Operations":[{"op":" add"}]}"#, ResourceKind::Patch)]
#[case::unknown_op(r#"{"Operations":[{"op":"delete"}]}"#, ResourceKind::Patch)]
#[case::number_op(r#"{"Operations":[{"op":1}]}"#, ResourceKind::Patch)]
#[case::missing_operations(r#"{"schemas":[]}"#, ResourceKind::Patch)]
#[case::attribute_number(r#"{"schemas":[],"roles":[7]}"#, ResourceKind::User)]
#[case::malformed_json(r#"{"schemas":[]"#, ResourceKind::User)]
fn rejects_what_pydantic_rejects(#[case] body: &str, #[case] kind: ResourceKind) {
    decode_scim_resource(body.as_bytes(), kind).unwrap_err();
}
#[rstest]
#[case::no_dot("a@host")]
#[case::literal("a@[1.2.3.4]")]
#[case::newline(r"a@b.co\n")]
#[case::no_at("invalid")]
#[case::quoted(r#"\"a\"@b.co"#)]
#[case::test_tld("a@b.test")]
#[case::numeric_tld("a@b.123")]
fn rejects_emails_pydantic_rejects(#[case] sent: &str) {
    let body = format!(r#"{{"schemas":[],"emails":[{{"value":"{sent}"}}]}}"#);
    decode_scim_resource(body.as_bytes(), ResourceKind::User).unwrap_err();
}
#[rstest]
#[case::mixed_case_domain("Ada.Lovelace@EXAMPLE.Com", "Ada.Lovelace@example.com")]
#[case::padded(" ada@example.com ", "ada@example.com")]
#[case::display_name("Ada <Ada@Example.COM>", "Ada@example.com")]
#[case::shared_mailbox("PostMaster@Example.com", "postmaster@example.com")]
fn email_keeps_local_part_and_lowercases_domain(#[case] sent: &str, #[case] stored: &str) {
    let body = format!(r#"{{"schemas":[],"emails":[{{"value":"{sent}"}}]}}"#);
    let emails = user(&body).emails.unwrap();
    assert_eq!(emails.first().unwrap().value.0, stored);
}
#[rstest]
#[case::absent_blocks(
    ResourceKind::User,
    r#"{"schemas":[],"groups":[{"value":"g1"},{"value":"g2","type":null}]}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":[{"value":"g1","display":null,"type":"direct"},{"value":"g2","display":null,"type":null}]}"#
)]
#[case::bare_and_extra_attributes(
    ResourceKind::User,
    r#"{"schemas":[],"entitlements":["admin",{"value":"b","source":{"hr":[1]},"primary":"yes"}],"roles":[]}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":null,"entitlements":[{"value":"admin","display":null,"type":null,"primary":null},{"value":"b","display":null,"type":null,"primary":true,"source":{"hr":[1]}}],"roles":[]}"#
)]
#[case::member_type_number(
    ResourceKind::Group,
    r#"{"schemas":[],"displayName":"G","members":[{"value":"u1","type":4},{"value":"u2","type":"User"}]}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"displayName":"G","members":[{"value":"u1","display":null,"type":null},{"value":"u2","display":null,"type":"User"}]}"#
)]
#[case::patch_mixed_values(
    ResourceKind::Patch,
    r#"{"Operations":[{"op":"Add","path":"members","value":[{"value":"u1"},"u2",3]},{"op":"REPLACE","value":{"active":"False"}},{"op":"remove","value":null}]}"#,
    r#"{"schemas":["urn:ietf:params:scim:api:messages:2.0:PatchOp"],"Operations":[{"op":"add","path":"members","value":[{"value":"u1"},"u2",3]},{"op":"replace","path":null,"value":{"active":"False"}},{"op":"remove","path":null,"value":null}]}"#
)]
#[case::enterprise_urn_and_ref(
    ResourceKind::User,
    r#"{"schemas":[],"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"department":"R&D","manager":{"$ref":"/Users/1"}}}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":null,"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"employeeNumber":null,"costCenter":null,"organization":null,"division":null,"department":"R&D","manager":{"value":null,"displayName":null,"$ref":"/Users/1"}}}"#
)]
#[case::enterprise_field_names(
    ResourceKind::User,
    r#"{"schemas":[],"enterprise_user":{"department":"R&D","manager":{"ref":"/Users/1"}}}"#,
    r#"{"schemas":[],"id":null,"externalId":null,"meta":null,"userName":null,"name":null,"displayName":null,"active":true,"emails":null,"groups":null,"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"employeeNumber":null,"costCenter":null,"organization":null,"division":null,"department":"R&D","manager":{"value":null,"displayName":null,"$ref":"/Users/1"}}}"#
)]
fn dumps_match_pydantic(#[case] kind: ResourceKind, #[case] body: &str, #[case] expected: &str) {
    assert_eq!(dump(body, kind), expected);
}
#[test]
fn resource_type_renames_only_its_own_schema_field() {
    let body = r#"{"id":"User","name":"User","endpoint":"/Users","schema_":"core","schemaExtensions":[{"schema_":"ext","required":false}]}"#;
    let record: ResourceType = serde_json::from_str(body).unwrap();
    let expected = r#"{"schemas":["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],"id":"User","name":"User","description":null,"endpoint":"/Users","schema":"core","schemaExtensions":[{"schema_":"ext","required":false}],"meta":null}"#;
    assert_eq!(serde_json::to_string(&record).unwrap(), expected);
    let bare = r#"{"id":"G","name":"G","endpoint":"/Groups","schema_":"core"}"#;
    let dumped = serde_json::to_string(&serde_json::from_str::<ResourceType>(bare).unwrap());
    assert!(!dumped.unwrap().contains("schemaExtensions"));
}
#[test]
fn schema_keeps_nested_null_sub_attributes() {
    let body = r#"{"id":"User","name":"User","attributes":[{"name":"manager","type":"complex","subAttributes":[{"name":"value","type":"string"}]}]}"#;
    let record: Schema = serde_json::from_str(body).unwrap();
    let expected = r#"{"schemas":["urn:ietf:params:scim:schemas:core:2.0:Schema"],"id":"User","name":"User","description":null,"attributes":[{"name":"manager","type":"complex","multiValued":false,"description":null,"required":false,"mutability":"readWrite","returned":"default","uniqueness":"none","subAttributes":[{"name":"value","type":"string","multiValued":false,"description":null,"required":false,"mutability":"readWrite","returned":"default","uniqueness":"none","subAttributes":null}]}],"meta":null}"#;
    assert_eq!(serde_json::to_string(&record).unwrap(), expected);
}
#[test]
fn service_provider_config_supports_only_patch() {
    let off = r#"{"supported":false,"maxOperations":null,"maxPayloadSize":null,"maxResults":null}"#;
    let expected = format!(
        r#"{{"schemas":["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],"patch":{{"supported":true,"maxOperations":null,"maxPayloadSize":null,"maxResults":null}},"bulk":{off},"filter":{off},"changePassword":{off},"sort":{off},"etag":{off},"authenticationSchemes":null,"meta":null}}"#
    );
    let dumped = serde_json::to_string(&ServiceProviderConfig::default());
    assert_eq!(dumped.unwrap(), expected);
}
#[rstest]
fn replays_the_scim_resource_corpus(#[files("fuzz/corpus/scim_resource/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let kind = match bytes.first() {
        Some(b'u') => ResourceKind::User,
        Some(b'g') => ResourceKind::Group,
        _ => ResourceKind::Patch,
    };
    drop(decode_scim_resource(
        bytes.get(1..).unwrap_or_default(),
        kind,
    ));
}
