use indexmap::IndexMap;
use rstest::rstest;

use super::support::{DEFAULT_BASE, anthropic_config, as_json, serialized};
use crate::skills::config::BaseSkillsApiConfig as _;
use crate::skills::wire::{
    CreateSkillRequest, DeleteSkillResponse, ListSkillsParams, ListSkillsResponse, Skill,
};

const MINIMAL_SKILL: &str = r#"{"id":"skill_1","created_at":"2026-01-01T00:00:00Z","source":"custom","updated_at":"2026-01-02T00:00:00Z"}"#;

#[test]
fn skills_wire_types_create_request_round_trips_title_files_and_extras() {
    let body = r#"{"display_title":"Title","files":[{"path":"SKILL.md"}],"description":"Extra"}"#;
    let request: CreateSkillRequest = serde_json::from_str(body).unwrap();
    assert_eq!(request.display_title.as_deref(), Some("Title"));
    assert_eq!(
        serialized(&request.files),
        as_json(r#"[{"path":"SKILL.md"}]"#)
    );
    assert_eq!(
        serialized(&request.extra),
        as_json(r#"{"description":"Extra"}"#)
    );
    assert_eq!(
        serialized(&request),
        as_json(r#"{"display_title":"Title","files":[{"path":"SKILL.md"}],"description":"Extra"}"#)
    );
}

#[test]
fn skills_wire_types_create_request_drops_absent_fields_on_serialize() {
    let request = CreateSkillRequest {
        display_title: None,
        files: None,
        extra: IndexMap::new(),
    };
    assert_eq!(serialized(&request), as_json("{}"));
}

#[test]
fn skills_wire_types_create_transform_drops_null_extras_and_keeps_an_empty_file_list() {
    let body = r#"{"display_title":"Title","files":[],"description":"kept","ignored":null}"#;
    let request: CreateSkillRequest = serde_json::from_str(body).unwrap();
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let transformed = config.transform_create_skill_request(&request);
    assert_eq!(
        serialized(&transformed),
        as_json(r#"{"display_title":"Title","files":[],"description":"kept"}"#)
    );
}

#[test]
fn skills_wire_types_list_params_round_trip() {
    let params: ListSkillsParams =
        serde_json::from_str(r#"{"limit":5,"page":"tok","source":"custom"}"#).unwrap();
    assert_eq!(
        params,
        ListSkillsParams {
            limit: Some(5),
            page: Some("tok".to_owned()),
            source: Some("custom".to_owned()),
        }
    );
    assert_eq!(
        serialized(&params),
        as_json(r#"{"limit":5,"page":"tok","source":"custom"}"#)
    );
    let absent: ListSkillsParams = serde_json::from_str("{}").unwrap();
    assert_eq!(absent, ListSkillsParams::default());
    assert_eq!(serialized(&absent), as_json("{}"));
}

#[test]
fn skills_wire_types_skill_defaults_type_and_optional_fields() {
    let skill = serde_json::from_slice::<Skill>(MINIMAL_SKILL.as_bytes()).unwrap();
    assert_eq!(skill.id, "skill_1");
    assert_eq!(skill.created_at, "2026-01-01T00:00:00Z");
    assert_eq!(skill.source, "custom");
    assert_eq!(skill.updated_at, "2026-01-02T00:00:00Z");
    assert_eq!(skill.r#type, "skill");
    assert_eq!(skill.display_title, None);
    assert_eq!(skill.latest_version, None);
    assert_eq!(skill.description, None);
    assert_eq!(skill.search_score, None);
}

#[test]
fn skills_wire_types_skill_serializes_every_declared_field() {
    let minimal = serde_json::from_slice::<Skill>(MINIMAL_SKILL.as_bytes()).unwrap();
    assert_eq!(
        serialized(&minimal),
        as_json(
            r#"{"id":"skill_1","created_at":"2026-01-01T00:00:00Z","display_title":null,"latest_version":null,"source":"custom","type":"skill","updated_at":"2026-01-02T00:00:00Z","description":null,"search_score":null}"#
        )
    );
    let full = serde_json::from_slice::<Skill>(
        br#"{"id":"skill_2","created_at":"a","display_title":"T","latest_version":"v1","source":"anthropic","type":"skill","updated_at":"b","description":"D","search_score":0.5}"#,
    )
    .unwrap();
    assert_eq!(
        serialized(&full),
        as_json(
            r#"{"id":"skill_2","created_at":"a","display_title":"T","latest_version":"v1","source":"anthropic","type":"skill","updated_at":"b","description":"D","search_score":0.5}"#
        )
    );
}

#[test]
fn skills_wire_types_skill_ignores_unknown_fields_and_keeps_null_optionals() {
    let skill = serde_json::from_slice::<Skill>(
        br#"{"id":"skill_1","created_at":"a","source":"custom","updated_at":"b","display_title":null,"unknown":1}"#,
    )
    .unwrap();
    assert_eq!(skill.r#type, "skill");
    assert_eq!(skill.display_title, None);
}

#[test]
fn skills_wire_types_skill_rejects_a_null_type() {
    let _error = serde_json::from_slice::<Skill>(
        br#"{"id":"skill_1","created_at":"a","source":"custom","updated_at":"b","type":null}"#,
    )
    .unwrap_err();
}

#[rstest]
#[case::id(r#"{"created_at":"a","source":"custom","updated_at":"b"}"#, "id")]
#[case::created_at(r#"{"id":"s","source":"custom","updated_at":"b"}"#, "created_at")]
#[case::source(r#"{"id":"s","created_at":"a","updated_at":"b"}"#, "source")]
#[case::updated_at(r#"{"id":"s","created_at":"a","source":"custom"}"#, "updated_at")]
fn skills_wire_types_skill_rejects_missing_required_fields(
    #[case] body: &str,
    #[case] field: &str,
) {
    let error = serde_json::from_slice::<Skill>(body.as_bytes()).unwrap_err();
    assert!(error.to_string().contains(field), "{error}");
}

#[rstest]
#[case::id_number(r#"{"id":7,"created_at":"a","source":"custom","updated_at":"b"}"#)]
#[case::source_list(r#"{"id":"s","created_at":"a","source":[],"updated_at":"b"}"#)]
#[case::not_an_object("[]")]
fn skills_wire_types_skill_rejects_wrong_types(#[case] body: &str) {
    let _error = serde_json::from_slice::<Skill>(body.as_bytes()).unwrap_err();
}

#[test]
fn skills_wire_types_list_response_defaults_next_page_and_has_more() {
    let list = serde_json::from_slice::<ListSkillsResponse>(
        br#"{"data":[{"id":"skill_1","created_at":"2026-01-01T00:00:00Z","source":"custom","updated_at":"2026-01-02T00:00:00Z"}]}"#,
    )
    .unwrap();
    assert_eq!(list.data.len(), 1);
    assert_eq!(list.data[0].id, "skill_1");
    assert_eq!(list.next_page, None);
    assert!(!list.has_more);
    let full = serde_json::from_slice::<ListSkillsResponse>(
        br#"{"data":[],"next_page":"p2","has_more":true,"unknown":1}"#,
    )
    .unwrap();
    assert_eq!(full.next_page.as_deref(), Some("p2"));
    assert!(full.has_more);
    let null_page =
        serde_json::from_slice::<ListSkillsResponse>(br#"{"data":[],"next_page":null}"#).unwrap();
    assert_eq!(null_page.next_page, None);
    assert!(!null_page.has_more);
    let _error = serde_json::from_slice::<ListSkillsResponse>(br#"{"data":[],"has_more":null}"#)
        .unwrap_err();
}

#[test]
fn skills_wire_types_list_response_requires_data() {
    let error = serde_json::from_slice::<ListSkillsResponse>(br#"{"next_page":null}"#).unwrap_err();
    assert!(error.to_string().contains("data"), "{error}");
}

#[test]
fn skills_wire_types_delete_response_defaults_the_type() {
    let deleted = serde_json::from_slice::<DeleteSkillResponse>(br#"{"id":"skill_1"}"#).unwrap();
    assert_eq!(
        deleted,
        DeleteSkillResponse {
            id: "skill_1".to_owned(),
            r#type: "skill_deleted".to_owned(),
        }
    );
    let typed =
        serde_json::from_slice::<DeleteSkillResponse>(br#"{"id":"skill_2","type":"custom"}"#)
            .unwrap();
    assert_eq!(typed.r#type, "custom");
    let _error = serde_json::from_slice::<DeleteSkillResponse>(br#"{"id":"skill_3","type":null}"#)
        .unwrap_err();
}

#[test]
fn skills_wire_types_response_transforms_map_each_reply() {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let created = config
        .transform_create_skill_response(
            br#"{"id":"s1","created_at":"a","source":"custom","updated_at":"b"}"#,
        )
        .unwrap();
    let fetched = config
        .transform_get_skill_response(
            br#"{"id":"s2","created_at":"a","source":"custom","updated_at":"b"}"#,
        )
        .unwrap();
    let list = config
        .transform_list_skills_response(
            br#"{"data":[{"id":"skill_1","created_at":"2026-01-01T00:00:00Z","source":"custom","updated_at":"2026-01-02T00:00:00Z"}]}"#,
        )
        .unwrap();
    let deleted = config
        .transform_delete_skill_response(br#"{"id":"s3"}"#)
        .unwrap();
    assert_eq!(created.id, "s1");
    assert_eq!(fetched.id, "s2");
    assert_eq!(list.data.len(), 1);
    assert_eq!(deleted.id, "s3");
}

#[rstest]
#[case::truncated(r#"{"id":"#)]
#[case::not_json("not json")]
fn skills_wire_types_response_transforms_reject_malformed_json(#[case] body: &str) {
    let config = anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let _created = config
        .transform_create_skill_response(body.as_bytes())
        .unwrap_err();
    let _fetched = config
        .transform_get_skill_response(body.as_bytes())
        .unwrap_err();
    let _listed = config
        .transform_list_skills_response(body.as_bytes())
        .unwrap_err();
    let _deleted = config
        .transform_delete_skill_response(body.as_bytes())
        .unwrap_err();
}
