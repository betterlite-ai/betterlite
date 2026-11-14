#![cfg(test)]

use super::content_type::set_scim_content_type;
use super::directory::{
    Directory, Metadata, is_attribute_list, is_object, parse_directory_metadata,
};
use super::error::ProjectionError;
use super::group::project_group;
use super::lookups::ScimLookups;
use super::member::{project_member, team_member_user_ids};
use super::naming::scim_names;
use super::timestamps::{isoformat, resource_meta};
use super::truthiness::is_set;
use super::user::{ScimUserSource, UserProjection, project_user};
use crate::scim_records::user::{GroupKind, UserGroup};
use crate::scim_records::{
    ENTERPRISE_METADATA_KEY, ENTERPRISE_USER_SCHEMA, ENTITLEMENTS_METADATA_KEY, ROLES_METADATA_KEY,
};
use axum::body::Body;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use http::StatusCode;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use tokio_postgres::NoTls;

const ENTERPRISE_FIELD: &str = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User";

fn json_data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn as_json<T>(value: &T) -> JsonData
where
    T: Serialize,
{
    json_data(&serde_json::to_string(value).unwrap())
}

fn field<'a>(value: &'a JsonData, name: &str) -> &'a JsonData {
    let JsonData::Object(fields) = value else {
        panic!("{name} read from a non-object");
    };
    match fields.get(name) {
        Some(found) => found,
        None => panic!("{name} is missing"),
    }
}

fn element<'a>(value: &'a JsonData, index: usize) -> &'a JsonData {
    let JsonData::Array(items) = value else {
        panic!("index {index} read from a non-array");
    };
    &items[index]
}

fn metadata(text: &str) -> Metadata {
    serde_json::from_str(text).unwrap()
}

fn stored_user(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).unwrap()
}

fn stored_team(text: &str) -> LiteLLMTeamTable {
    serde_json::from_str(text).unwrap()
}

fn aware(text: &str) -> Timestamp {
    Timestamp::Aware(DateTime::parse_from_rfc3339(text).unwrap())
}

fn naive(text: &str) -> Timestamp {
    Timestamp::Naive(NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").unwrap())
}

fn projected(text: &str) -> UserProjection {
    let user = stored_user(text);
    project_user(&ScimUserSource::from(&user), Vec::new()).unwrap()
}

fn projected_json(text: &str) -> JsonData {
    as_json(&projected(text).user)
}

fn user_with_metadata(stored: &str) -> String {
    format!(r#"{{"user_id":"u","metadata":{stored}}}"#)
}

#[rstest]
#[case::null("null", false)]
#[case::true_flag("true", true)]
#[case::false_flag("false", false)]
#[case::zero("0", false)]
#[case::zero_float("0.0", false)]
#[case::one("1", true)]
#[case::negative("-1", true)]
#[case::empty_string("\"\"", false)]
#[case::text("\"no\"", true)]
#[case::empty_array("[]", false)]
#[case::array_with_zero("[0]", true)]
#[case::empty_object("{}", false)]
#[case::object("{\"a\":0}", true)]
fn scim_projections_truthiness_follows_stored_value(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(is_set(&json_data(text)), expected);
}

#[rstest]
#[case::object("{}", true)]
#[case::array("[]", false)]
#[case::text("\"x\"", false)]
#[case::number("1", false)]
#[case::flag("true", false)]
#[case::null("null", false)]
fn scim_projections_object_guard_accepts_only_objects(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(is_object(&json_data(text)), expected);
}

#[rstest]
#[case::strings("[\"a\",\"b\"]", true)]
#[case::objects("[{\"value\":\"a\"}]", true)]
#[case::mixed("[\"a\",{\"value\":\"b\"}]", true)]
#[case::empty("[]", true)]
#[case::nested_array("[[\"a\"]]", false)]
#[case::number_item("[1]", false)]
#[case::null_item("[null]", false)]
#[case::flag_item("[true]", false)]
#[case::text("\"a\"", false)]
#[case::object("{}", false)]
#[case::number("1", false)]
#[case::flag("true", false)]
#[case::null("null", false)]
fn scim_projections_list_guard_accepts_only_attribute_lists(
    #[case] text: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_attribute_list(&json_data(text)), expected);
}

fn directory_label(stored: &str) -> &'static str {
    let found = metadata(&format!(r#"{{"key":{stored}}}"#));
    match parse_directory_metadata::<Vec<String>>(&found, "key", is_attribute_list).unwrap() {
        Directory::Absent => "absent",
        Directory::Parsed(_) => "parsed",
        Directory::Malformed => "malformed",
    }
}

#[rstest]
#[case::null("null", "absent")]
#[case::empty_array("[]", "absent")]
#[case::empty_text("\"\"", "absent")]
#[case::zero("0", "absent")]
#[case::false_flag("false", "absent")]
#[case::empty_object("{}", "absent")]
#[case::valid("[\"a\"]", "parsed")]
#[case::fails_guard("[1]", "malformed")]
#[case::wrong_shape("\"text\"", "malformed")]
#[case::fails_decode("[{\"value\":\"a\"}]", "malformed")]
fn scim_projections_directory_parse_outcomes(#[case] stored: &str, #[case] expected: &str) {
    assert_eq!(directory_label(stored), expected);
}

#[test]
fn scim_projections_directory_parse_of_a_missing_key_is_absent() {
    let found = Metadata::new();
    let outcome = parse_directory_metadata::<Vec<String>>(&found, "key", is_attribute_list);
    assert!(matches!(outcome.unwrap(), Directory::Absent));
}

#[rstest]
#[case::nothing(
    None,
    None,
    "{}",
    ("Unknown Display Name", "Unknown User", "Unknown Family Name")
)]
#[case::email_only(
    Some("a@b.co"),
    None,
    "{}",
    ("a@b.co", "Unknown User", "Unknown Family Name")
)]
#[case::alias_only(Some(""), Some("Ann"), "{}", ("Unknown Display Name", "Ann", "Ann"))]
#[case::email_beats_alias(Some("a@b.co"), Some("Ann"), "{}", ("a@b.co", "Ann", "Ann"))]
#[case::empty_alias(
    Some("a@b.co"),
    Some(""),
    "{}",
    ("a@b.co", "Unknown User", "Unknown Family Name")
)]
#[case::uuid_email(
    Some("3f2504e0-4f89"),
    None,
    "{}",
    ("3f2504e0-4f89", "Unknown User", "Unknown Family Name")
)]
#[case::stored_names_beat_alias(
    None,
    Some("Ann"),
    r#"{"scim_metadata":{"givenName":"Gi","familyName":"Fa"}}"#,
    ("Unknown Display Name", "Gi", "Fa")
)]
#[case::empty_stored_names_use_alias(
    None,
    Some("Ann"),
    r#"{"scim_metadata":{"givenName":"","familyName":""}}"#,
    ("Unknown Display Name", "Ann", "Ann")
)]
#[case::given_only(
    None,
    Some("Ann"),
    r#"{"scim_metadata":{"givenName":"Gi"}}"#,
    ("Unknown Display Name", "Gi", "Ann")
)]
#[case::null_stored_names(
    None,
    None,
    r#"{"scim_metadata":{"givenName":null,"familyName":null}}"#,
    ("Unknown Display Name", "Unknown User", "Unknown Family Name")
)]
fn scim_projections_names_prefer_stored_values(
    #[case] email: Option<&str>,
    #[case] alias: Option<&str>,
    #[case] stored: &str,
    #[case] expected: (&str, &str, &str),
) {
    let names = scim_names(email, alias, &metadata(stored)).unwrap();
    let projected = (
        names.user_name.as_str(),
        names.given_name.as_str(),
        names.family_name.as_str(),
    );
    assert_eq!(projected, expected);
}

#[rstest]
#[case::null(r#"{"scim_metadata":null}"#)]
#[case::text(r#"{"scim_metadata":"Ann"}"#)]
#[case::array(r#"{"scim_metadata":[]}"#)]
#[case::number(r#"{"scim_metadata":1}"#)]
#[case::flag(r#"{"scim_metadata":false}"#)]
fn scim_projections_names_reject_a_scim_metadata_that_is_not_an_object(#[case] stored: &str) {
    let error = scim_names(None, None, &metadata(stored)).err().unwrap();
    assert!(matches!(error, ProjectionError::ScimMetadataShape));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn scim_projections_names_reject_a_scim_metadata_name_that_is_not_text() {
    let stored = metadata(r#"{"scim_metadata":{"givenName":5}}"#);
    let error = scim_names(None, None, &stored).err().unwrap();
    assert!(matches!(error, ProjectionError::ScimMetadata { .. }));
}

#[rstest]
#[case::both(Some("u1"), Some("a@b.co"), "u1", "a@b.co")]
#[case::id_only(Some("u1"), None, "u1", "u1")]
#[case::empty_email(Some("u1"), Some(""), "u1", "u1")]
#[case::email_only(None, Some("a@b.co"), "Unknown Member Value", "a@b.co")]
#[case::nothing(None, None, "Unknown Member Value", "Unknown Member Value")]
#[case::empty_id(Some(""), None, "Unknown Member Value", "Unknown Member Value")]
#[case::empty_id_with_email(Some(""), Some("a@b.co"), "Unknown Member Value", "a@b.co")]
fn scim_projections_member_value_and_display(
    #[case] user_id: Option<&str>,
    #[case] email: Option<&str>,
    #[case] value: &str,
    #[case] display: &str,
) {
    let member = project_member(user_id, email);
    assert_eq!(member.value, value);
    assert_eq!(member.display.as_deref(), Some(display));
    assert_eq!(member.kind.0.as_deref(), Some("User"));
}

#[rstest]
#[case::no_microseconds("2026-01-02T03:04:05", "2026-01-02T03:04:05")]
#[case::microseconds("2026-01-02T03:04:05.123456", "2026-01-02T03:04:05.123456")]
#[case::below_a_microsecond("2026-01-02T03:04:05.000000999", "2026-01-02T03:04:05")]
#[case::one_microsecond("2026-01-02T03:04:05.000001", "2026-01-02T03:04:05.000001")]
#[case::milliseconds("2026-01-02T03:04:05.5", "2026-01-02T03:04:05.500000")]
fn scim_projections_naive_isoformat_has_no_offset(#[case] stored: &str, #[case] expected: &str) {
    assert_eq!(isoformat(naive(stored)), expected);
}

#[rstest]
#[case::utc("2026-01-02T03:04:05Z", "2026-01-02T03:04:05+00:00")]
#[case::utc_microseconds("2026-01-02T03:04:05.123456Z", "2026-01-02T03:04:05.123456+00:00")]
#[case::positive_offset("2026-01-02T03:04:05.250+05:30", "2026-01-02T03:04:05.250000+05:30")]
#[case::negative_offset("2026-01-02T03:04:05-08:00", "2026-01-02T03:04:05-08:00")]
#[case::below_a_microsecond("2026-01-02T03:04:05.000000999Z", "2026-01-02T03:04:05+00:00")]
fn scim_projections_aware_isoformat_keeps_the_offset(#[case] stored: &str, #[case] expected: &str) {
    assert_eq!(isoformat(aware(stored)), expected);
}

#[test]
fn scim_projections_meta_names_the_resource_and_both_times() {
    let meta = resource_meta(
        "User",
        Some(naive("2026-01-02T03:04:05")),
        Some(aware("2026-01-03T03:04:05Z")),
    )
    .unwrap();
    let expected = json_data(
        r#"{"resourceType":"User","created":"2026-01-02T03:04:05","lastModified":"2026-01-03T03:04:05+00:00"}"#,
    );
    assert_eq!(as_json(&meta), expected);
}

#[test]
fn scim_projections_meta_without_times_has_nulls() {
    let meta = resource_meta("Group", None, None).unwrap();
    let expected = json_data(r#"{"resourceType":"Group","created":null,"lastModified":null}"#);
    assert_eq!(as_json(&meta), expected);
}

#[test]
fn scim_projections_uuid_email_is_still_the_username() {
    let user = projected_json(
        r#"{"user_id":"u","user_email":"3f2504e0-4f89-11d3-9a0c-0305e82c3301","user_alias":"Ann"}"#,
    );
    let address = json_data(r#""3f2504e0-4f89-11d3-9a0c-0305e82c3301""#);
    assert_eq!(field(&user, "userName"), &address);
    assert_eq!(field(&user, "displayName"), &address);
    assert_eq!(field(&user, "emails"), &json_data("[]"));
    assert_eq!(
        field(field(&user, "name"), "givenName"),
        &json_data(r#""Ann""#)
    );
    assert_eq!(
        field(field(&user, "name"), "familyName"),
        &json_data(r#""Ann""#)
    );
}

#[test]
fn scim_projections_address_with_at_sign_is_an_email_and_the_username() {
    let user = projected_json(r#"{"user_id":"u","user_email":"ann@example.com"}"#);
    assert_eq!(field(&user, "userName"), &json_data(r#""ann@example.com""#));
    assert_eq!(
        field(&user, "displayName"),
        &json_data(r#""ann@example.com""#)
    );
    let emails = field(&user, "emails");
    assert_eq!(
        field(element(emails, 0), "value"),
        &json_data(r#""ann@example.com""#)
    );
    assert_eq!(field(element(emails, 0), "primary"), &JsonData::Bool(true));
    assert_eq!(field(&user, "id"), &json_data(r#""u""#));
    let schemas = json_data(&format!(r#"["{}"]"#, super::USER_SCHEMA));
    assert_eq!(field(&user, "schemas"), &schemas);
}

#[test]
fn scim_projections_user_without_email_or_alias_gets_placeholders() {
    let user = projected_json(r#"{"user_id":"u"}"#);
    assert_eq!(
        field(&user, "userName"),
        &json_data(r#""Unknown Display Name""#)
    );
    assert_eq!(
        field(&user, "displayName"),
        &json_data(r#""Unknown Display Name""#)
    );
    assert_eq!(
        field(field(&user, "name"), "givenName"),
        &json_data(r#""Unknown User""#)
    );
    assert_eq!(
        field(field(&user, "name"), "familyName"),
        &json_data(r#""Unknown Family Name""#)
    );
    assert_eq!(field(&user, "emails"), &json_data("[]"));
    assert_eq!(field(&user, "groups"), &json_data("[]"));
}

#[test]
fn scim_projections_invalid_address_with_at_sign_fails_the_projection() {
    let user = stored_user(r#"{"user_id":"u","user_email":"@no-local"}"#);
    let error = project_user(&ScimUserSource::from(&user), Vec::new())
        .err()
        .unwrap();
    assert!(matches!(error, ProjectionError::Email { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn scim_projections_malformed_scim_metadata_keeps_its_error() {
    let user = stored_user(&user_with_metadata(r#"{"scim_metadata":"Ann"}"#));
    let error = project_user(&ScimUserSource::from(&user), Vec::new())
        .err()
        .unwrap();
    assert!(matches!(error, ProjectionError::ScimMetadataShape));
}

#[test]
fn scim_projections_user_without_metadata_is_active() {
    let active = JsonData::Bool(true);
    assert_eq!(
        field(&projected_json(r#"{"user_id":"u"}"#), "active"),
        &active
    );
    assert_eq!(
        field(
            &projected_json(r#"{"user_id":"u","metadata":null}"#),
            "active"
        ),
        &active
    );
    assert_eq!(
        field(&projected_json(&user_with_metadata("{}")), "active"),
        &active
    );
}

#[rstest]
#[case::null("null", true)]
#[case::true_flag("true", true)]
#[case::false_flag("false", false)]
#[case::zero("0", false)]
#[case::one("1", true)]
#[case::empty_text("\"\"", false)]
#[case::text("\"no\"", true)]
#[case::empty_array("[]", false)]
#[case::empty_object("{}", false)]
fn scim_projections_active_follows_scim_active(#[case] stored: &str, #[case] expected: bool) {
    let text = user_with_metadata(&format!(r#"{{"scim_active":{stored}}}"#));
    assert_eq!(
        field(&projected_json(&text), "active"),
        &JsonData::Bool(expected)
    );
}

#[rstest]
#[case::enterprise_object(
    ENTERPRISE_METADATA_KEY,
    ENTERPRISE_FIELD,
    r#"{"employeeNumber":"7"}"#,
    false,
    true
)]
#[case::enterprise_array(ENTERPRISE_METADATA_KEY, ENTERPRISE_FIELD, "[1]", true, false)]
#[case::enterprise_text(ENTERPRISE_METADATA_KEY, ENTERPRISE_FIELD, r#""x""#, true, false)]
#[case::enterprise_number(ENTERPRISE_METADATA_KEY, ENTERPRISE_FIELD, "3", true, false)]
#[case::enterprise_field_type(
    ENTERPRISE_METADATA_KEY,
    ENTERPRISE_FIELD,
    r#"{"employeeNumber":5}"#,
    true,
    false
)]
#[case::enterprise_empty(ENTERPRISE_METADATA_KEY, ENTERPRISE_FIELD, "{}", false, false)]
#[case::enterprise_null(ENTERPRISE_METADATA_KEY, ENTERPRISE_FIELD, "null", false, false)]
#[case::enterprise_zero(ENTERPRISE_METADATA_KEY, ENTERPRISE_FIELD, "0", false, false)]
#[case::entitlements_text(ENTITLEMENTS_METADATA_KEY, "entitlements", r#"["read"]"#, false, true)]
#[case::entitlements_object(
    ENTITLEMENTS_METADATA_KEY,
    "entitlements",
    r#"[{"value":"x"}]"#,
    false,
    true
)]
#[case::entitlements_nested(ENTITLEMENTS_METADATA_KEY, "entitlements", r#"[["x"]]"#, true, false)]
#[case::entitlements_text_value(
    ENTITLEMENTS_METADATA_KEY,
    "entitlements",
    r#""read""#,
    true,
    false
)]
#[case::entitlements_field_type(
    ENTITLEMENTS_METADATA_KEY,
    "entitlements",
    r#"[{"value":5}]"#,
    true,
    false
)]
#[case::entitlements_empty(ENTITLEMENTS_METADATA_KEY, "entitlements", "[]", false, false)]
#[case::roles_text(ROLES_METADATA_KEY, "roles", r#"["admin"]"#, false, true)]
#[case::roles_number(ROLES_METADATA_KEY, "roles", "7", true, false)]
#[case::roles_nested(ROLES_METADATA_KEY, "roles", "[[1]]", true, false)]
#[case::roles_empty(ROLES_METADATA_KEY, "roles", "[]", false, false)]
fn scim_projections_directory_blocks_are_kept_or_dropped(
    #[case] key: &'static str,
    #[case] field: &str,
    #[case] stored: &str,
    #[case] dropped: bool,
    #[case] present: bool,
) {
    let projection = projected(&user_with_metadata(&format!(r#"{{"{key}":{stored}}}"#)));
    let user = as_json(&projection.user);
    let JsonData::Object(fields) = &user else {
        panic!("the projected user is not a JSON object");
    };
    let expected_dropped: Vec<&str> = if dropped { vec![key] } else { Vec::new() };
    assert_eq!(projection.dropped, expected_dropped);
    assert_eq!(fields.contains_key(field), present);
}

#[test]
fn scim_projections_enterprise_block_adds_its_schema() {
    let user = projected_json(&user_with_metadata(&format!(
        r#"{{"{ENTERPRISE_METADATA_KEY}":{{"department":"R&D"}}}}"#
    )));
    let schemas = json_data(&format!(
        r#"["{}","{}"]"#,
        super::USER_SCHEMA,
        ENTERPRISE_USER_SCHEMA
    ));
    assert_eq!(field(&user, "schemas"), &schemas);
    assert_eq!(
        field(field(&user, ENTERPRISE_FIELD), "department"),
        &json_data(r#""R&D""#)
    );
}

#[test]
fn scim_projections_dropped_enterprise_block_keeps_the_core_schema_only() {
    let user = projected_json(&user_with_metadata(&format!(
        r#"{{"{ENTERPRISE_METADATA_KEY}":[1]}}"#
    )));
    let schemas = json_data(&format!(r#"["{}"]"#, super::USER_SCHEMA));
    assert_eq!(field(&user, "schemas"), &schemas);
}

#[test]
fn scim_projections_broken_directory_block_drops_only_that_block() {
    let projection = projected(&user_with_metadata(&format!(
        r#"{{"{ENTITLEMENTS_METADATA_KEY}":[[1]],"{ROLES_METADATA_KEY}":["admin"],"scim_active":false}}"#
    )));
    let user = as_json(&projection.user);
    let JsonData::Object(fields) = &user else {
        panic!("the projected user is not a JSON object");
    };
    assert_eq!(projection.dropped, vec![ENTITLEMENTS_METADATA_KEY]);
    assert!(!fields.contains_key("entitlements"));
    assert_eq!(
        field(element(field(&user, "roles"), 0), "value"),
        &json_data(r#""admin""#)
    );
    assert_eq!(field(&user, "active"), &JsonData::Bool(false));
}

#[test]
fn scim_projections_user_times_are_formatted_as_given() {
    let user = projected_json(
        r#"{"user_id":"u","created_at":"2026-01-02T03:04:05.123456","updated_at":"2026-01-02T03:04:06+00:00"}"#,
    );
    let meta = field(&user, "meta");
    assert_eq!(field(meta, "resourceType"), &json_data(r#""User""#));
    assert_eq!(
        field(meta, "created"),
        &json_data(r#""2026-01-02T03:04:05.123456""#)
    );
    assert_eq!(
        field(meta, "lastModified"),
        &json_data(r#""2026-01-02T03:04:06+00:00""#)
    );
}

#[test]
fn scim_projections_user_without_times_has_null_meta_times() {
    let user = projected_json(r#"{"user_id":"u"}"#);
    let meta = field(&user, "meta");
    assert_eq!(field(meta, "created"), &JsonData::Null);
    assert_eq!(field(meta, "lastModified"), &JsonData::Null);
}

#[test]
fn scim_projections_user_keeps_the_groups_it_is_given() {
    let user = stored_user(r#"{"user_id":"u"}"#);
    let groups = vec![UserGroup {
        value: "t1".to_owned(),
        display: Some("alpha".to_owned()),
        kind: GroupKind::default(),
    }];
    let projection = project_user(&ScimUserSource::from(&user), groups).unwrap();
    let user_json = as_json(&projection.user);
    assert_eq!(
        field(&user_json, "groups"),
        &json_data(r#"[{"value":"t1","display":"alpha","type":"direct"}]"#)
    );
}

#[test]
fn scim_projections_source_carries_the_stored_team_ids() {
    let user = stored_user(r#"{"user_id":"u","teams":["t1","t2"]}"#);
    assert_eq!(ScimUserSource::from(&user).teams, ["t1", "t2"]);
}

const TEAM: &str = r#"{
    "team_id":"t1",
    "team_alias":"alpha",
    "members_with_roles":[
        {"user_id":"u1","user_email":"ann@example.com","role":"admin"},
        {"user_id":"u2","role":"user"},
        {"user_email":"x@y.co","role":"user"},
        {"user_id":"","role":"user"}
    ],
    "created_at":"2026-01-02T03:04:05+00:00",
    "updated_at":"2026-01-02T03:04:06.5+00:00"
}"#;

#[test]
fn scim_projections_group_walks_the_stored_roster() {
    let group = as_json(&project_group(&stored_team(TEAM)).unwrap());
    let schemas = json_data(&format!(r#"["{}"]"#, super::GROUP_SCHEMA));
    assert_eq!(field(&group, "schemas"), &schemas);
    assert_eq!(field(&group, "id"), &json_data(r#""t1""#));
    assert_eq!(field(&group, "displayName"), &json_data(r#""alpha""#));
    let meta = field(&group, "meta");
    assert_eq!(field(meta, "resourceType"), &json_data(r#""Group""#));
    assert_eq!(
        field(meta, "created"),
        &json_data(r#""2026-01-02T03:04:05+00:00""#)
    );
    assert_eq!(
        field(meta, "lastModified"),
        &json_data(r#""2026-01-02T03:04:06.500000+00:00""#)
    );
    let members = json_data(
        r#"[{"value":"u1","display":"ann@example.com","type":"User"},{"value":"u2","display":"u2","type":"User"},{"value":"Unknown Member Value","display":"x@y.co","type":"User"},{"value":"Unknown Member Value","display":"Unknown Member Value","type":"User"}]"#,
    );
    assert_eq!(field(&group, "members"), &members);
}

#[test]
fn scim_projections_group_without_an_alias_fails_instead_of_using_the_id() {
    let team = stored_team(r#"{"team_id":"t1","members_with_roles":[]}"#);
    let error = project_group(&team).err().unwrap();
    assert!(matches!(error, ProjectionError::GroupDisplayName));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn scim_projections_group_without_times_has_null_meta_times() {
    let team = stored_team(r#"{"team_id":"t1","team_alias":"alpha"}"#);
    let group = as_json(&project_group(&team).unwrap());
    assert_eq!(field(field(&group, "meta"), "created"), &JsonData::Null);
    assert_eq!(field(&group, "members"), &json_data("[]"));
}

#[test]
fn scim_projections_member_ids_keep_empty_ids_and_skip_missing_ones() {
    assert_eq!(team_member_user_ids(&stored_team(TEAM)), ["u1", "u2", ""]);
}

#[test]
fn scim_projections_member_ids_of_an_empty_roster_are_empty() {
    let team = stored_team(r#"{"team_id":"t1"}"#);
    assert!(team_member_user_ids(&team).is_empty());
}

#[rstest]
#[case::no_database(
    ProjectionError::NoDatabase,
    StatusCode::INTERNAL_SERVER_ERROR,
    "No database connected"
)]
#[case::user_missing(ProjectionError::UserNotFound { user_id: "u1".to_owned() }, StatusCode::NOT_FOUND, "User not found with ID: u1")]
#[case::group_missing(ProjectionError::GroupNotFound { team_id: "t1".to_owned() }, StatusCode::NOT_FOUND, "Group not found with ID: t1")]
fn scim_projections_error_status_and_detail(
    #[case] error: ProjectionError,
    #[case] status: StatusCode,
    #[case] detail: &str,
) {
    assert_eq!(error.status_code(), status);
    assert_eq!(error.to_string(), detail);
}

#[test]
fn scim_projections_content_type_is_scim_json() {
    let mut response = Response::new(Body::empty());
    set_scim_content_type(&mut response);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/scim+json"))
    );
}

#[test]
fn scim_projections_content_type_replaces_an_earlier_one() {
    let mut response = Response::new(Body::empty());
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    set_scim_content_type(&mut response);
    let values: Vec<&HeaderValue> = response.headers().get_all(CONTENT_TYPE).iter().collect();
    assert_eq!(values, [&HeaderValue::from_static("application/scim+json")]);
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    user_alias TEXT,
    user_email TEXT,
    teams TEXT[] NOT NULL DEFAULT '{}',
    metadata JSONB,
    created_at TIMESTAMP(3),
    updated_at TIMESTAMP(3)
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT,
    members_with_roles JSONB NOT NULL DEFAULT '[]',
    created_at TIMESTAMP(3),
    updated_at TIMESTAMP(3)
);
INSERT INTO "LiteLLM_UserTable" (user_id, user_alias, user_email, teams, created_at, updated_at) VALUES
    ('u1', 'Ann', 'ann@example.com', ARRAY['t1', 'gone', 't_null'], '2026-01-02 03:04:05.123', '2026-01-02 03:04:06'),
    ('u2', NULL, NULL, '{}', NULL, NULL),
    ('u3', 'Cy', '', ARRAY['t_bad'], NULL, NULL);
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, members_with_roles, created_at, updated_at) VALUES
    ('t1', 'alpha', '[{"user_id":"u1","role":"admin"},{"user_id":"ghost","role":"user"},{"user_id":"u3","role":"user"}]', '2026-02-03 04:05:06.5', NULL),
    ('t_null', NULL, '[]', NULL, NULL),
    ('t_bad', 'broken', '[5]', NULL, NULL);
"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["scim-projection-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
    drop(client);
    (fixture, pool)
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

fn assert_no_database(error: &ProjectionError) {
    assert!(matches!(error, ProjectionError::NoDatabase));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(error.to_string(), "No database connected");
}

#[tokio::test]
async fn scim_projections_every_lookup_without_a_database_is_a_500() {
    let missing = PostgresStore { pool: None };
    let lookups = ScimLookups { store: &missing };
    let user = stored_user(r#"{"user_id":"u","teams":["t1"]}"#);
    let team = stored_team(r#"{"team_id":"t1","team_alias":"alpha"}"#);
    assert_no_database(&lookups.check_user_exists("u").await.err().unwrap());
    assert_no_database(&lookups.check_team_exists("t1").await.err().unwrap());
    assert_no_database(
        &lookups
            .project_stored_user(&ScimUserSource::from(&user))
            .await
            .err()
            .unwrap(),
    );
    assert_no_database(&lookups.project_stored_group(&team).err().unwrap());
    assert_no_database(&lookups.get_team_members_display(&[]).await.err().unwrap());
    assert_no_database(
        &lookups
            .get_team_members_display(&["u".to_owned()])
            .await
            .err()
            .unwrap(),
    );
}

#[tokio::test]
async fn scim_projections_check_user_exists_returns_the_row_with_utc_times() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let user = lookups.check_user_exists("u1").await.unwrap();
    assert_eq!(user.user_id, "u1");
    assert_eq!(user.created_at, Some(aware("2026-01-02T03:04:05.123Z")));
    assert_eq!(user.updated_at, Some(aware("2026-01-02T03:04:06Z")));
    let bare = lookups.check_user_exists("u2").await.unwrap();
    assert_eq!(bare.created_at, None);
    assert_eq!(bare.updated_at, None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_check_user_exists_reports_a_missing_user() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let error = ScimLookups { store: &store }
        .check_user_exists("nobody")
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ProjectionError::UserNotFound { .. }));
    assert_eq!(error.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(error.to_string(), "User not found with ID: nobody");
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_check_team_exists_returns_the_row_with_utc_times() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let team = lookups.check_team_exists("t1").await.unwrap();
    assert_eq!(team.fields.base.team_id, "t1");
    assert_eq!(
        team.fields.created_at,
        Some(aware("2026-02-03T04:05:06.5Z"))
    );
    assert_eq!(team.fields.updated_at, None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_check_team_exists_reports_a_missing_team() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let error = ScimLookups { store: &store }
        .check_team_exists("nobody")
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ProjectionError::GroupNotFound { .. }));
    assert_eq!(error.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(error.to_string(), "Group not found with ID: nobody");
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_check_team_exists_rejects_a_row_that_does_not_validate() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let error = ScimLookups { store: &store }
        .check_team_exists("t_bad")
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ProjectionError::TeamQuery { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_user_groups_come_from_their_teams_and_skip_deleted_ones() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let user = lookups.check_user_exists("u1").await.unwrap();
    let projection = lookups
        .project_stored_user(&ScimUserSource::from(&user))
        .await
        .unwrap();
    let user_json = as_json(&projection.user);
    assert_eq!(
        field(&user_json, "groups"),
        &json_data(
            r#"[{"value":"t1","display":"alpha","type":"direct"},{"value":"t_null","display":null,"type":"direct"}]"#
        )
    );
    assert_eq!(
        field(&user_json, "userName"),
        &json_data(r#""ann@example.com""#)
    );
    let meta = field(&user_json, "meta");
    assert_eq!(
        field(meta, "created"),
        &json_data(r#""2026-01-02T03:04:05.123000+00:00""#)
    );
    assert_eq!(
        field(meta, "lastModified"),
        &json_data(r#""2026-01-02T03:04:06+00:00""#)
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_user_without_teams_has_no_groups() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let user = lookups.check_user_exists("u2").await.unwrap();
    let projection = lookups
        .project_stored_user(&ScimUserSource::from(&user))
        .await
        .unwrap();
    assert_eq!(
        field(&as_json(&projection.user), "groups"),
        &json_data("[]")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_user_groups_read_the_team_row_without_validating_it() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let user = lookups.check_user_exists("u3").await.unwrap();
    let projection = lookups
        .project_stored_user(&ScimUserSource::from(&user))
        .await
        .unwrap();
    assert_eq!(
        field(&as_json(&projection.user), "groups"),
        &json_data(r#"[{"value":"t_bad","display":"broken","type":"direct"}]"#)
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_group_keeps_a_member_whose_user_row_is_gone() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let team = lookups.check_team_exists("t1").await.unwrap();
    let group = lookups.project_stored_group(&team).unwrap();
    let group_json = as_json(&group);
    assert_eq!(field(&group_json, "displayName"), &json_data(r#""alpha""#));
    assert_eq!(
        field(field(&group_json, "meta"), "created"),
        &json_data(r#""2026-02-03T04:05:06.500000+00:00""#)
    );
    assert_eq!(
        field(&group_json, "members"),
        &json_data(
            r#"[{"value":"u1","display":"u1","type":"User"},{"value":"ghost","display":"ghost","type":"User"},{"value":"u3","display":"u3","type":"User"}]"#
        )
    );
    assert_eq!(team_member_user_ids(&team), ["u1", "ghost", "u3"]);
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_members_display_resolves_current_users_and_omits_missing_ones() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let lookups = ScimLookups { store: &store };
    let ids = ["u1", "ghost", "u3", "u2"].map(str::to_owned);
    let members = lookups.get_team_members_display(&ids).await.unwrap();
    assert_eq!(
        as_json(&members),
        json_data(
            r#"[{"value":"u1","display":"ann@example.com","type":"User"},{"value":"u3","display":"u3","type":"User"},{"value":"u2","display":"u2","type":"User"}]"#
        )
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_projections_members_display_of_no_ids_is_empty() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let members = ScimLookups { store: &store }
        .get_team_members_display(&[])
        .await
        .unwrap();
    assert!(members.is_empty());
    close(fixture, pool).await;
}
