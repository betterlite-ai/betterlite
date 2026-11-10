use std::collections::BTreeMap;

use http::header::HeaderMap;
use secrecy::SecretString;

use super::support::{DEFAULT_BASE, anthropic_config};
use crate::skills::config::BaseSkillsApiConfig;
use crate::skills::error::SkillsError;
use crate::skills::wire::{
    CreateSkillRequest, DeleteSkillResponse, ListSkillsParams, ListSkillsResponse, Skill,
};

struct FixtureConfig;

#[rustfmt::skip]
impl BaseSkillsApiConfig for FixtureConfig {
    fn validate_environment(&self, _headers: &mut HeaderMap, _api_key: Option<&SecretString>, _api_base: Option<&str>) -> Result<(), SkillsError> { Err(SkillsError::MissingCredential) }

    fn get_complete_url(&self, _api_base: Option<&str>, _endpoint: &str, _skill_id: Option<&str>) -> Result<String, SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_create_skill_request(&self, _request: &CreateSkillRequest) -> CreateSkillRequest { CreateSkillRequest::default() }

    fn transform_create_skill_response(&self, _body: &[u8]) -> Result<Skill, SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_list_skills_request(&self, _params: &ListSkillsParams, _api_base: Option<&str>) -> Result<(String, Vec<(String, String)>), SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_list_skills_response(&self, _body: &[u8]) -> Result<ListSkillsResponse, SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_get_skill_request(&self, _skill_id: &str, _api_base: Option<&str>) -> Result<String, SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_get_skill_response(&self, _body: &[u8]) -> Result<Skill, SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_delete_skill_request(&self, _skill_id: &str, _api_base: Option<&str>) -> Result<String, SkillsError> { Err(SkillsError::MissingCredential) }

    fn transform_delete_skill_response(&self, _body: &[u8]) -> Result<DeleteSkillResponse, SkillsError> { Err(SkillsError::MissingCredential) }
}

#[test]
fn skills_config_trait_provides_the_base_error_class() {
    let error = FixtureConfig.get_error_class("upstream exploded", 429, &BTreeMap::new());
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "upstream exploded");
    assert_eq!(error.headers, None);
    assert_eq!(error.request.method, "POST");
    assert_eq!(error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.response.status_code, 429);
    assert_eq!(error.response.request, error.request);
    assert_eq!(error.body, None);
}

#[test]
fn skills_config_trait_keeps_supplied_error_headers() {
    let headers = BTreeMap::from([("retry-after".to_owned(), "1".to_owned())]);
    let error = FixtureConfig.get_error_class("slow down", 503, &headers);
    assert_eq!(error.status_code, 503);
    assert_eq!(
        error.headers,
        Some(vec![("retry-after".to_owned(), "1".to_owned())])
    );
}

#[test]
fn skills_config_trait_declares_every_transformation() {
    let config: &dyn BaseSkillsApiConfig = &FixtureConfig;
    let _error = config.get_complete_url(None, "skills", None).unwrap_err();
    assert_eq!(
        config.transform_create_skill_request(&CreateSkillRequest::default()),
        CreateSkillRequest::default()
    );
}

#[test]
fn skills_config_trait_anthropic_implements_every_transformation() {
    let config: &dyn BaseSkillsApiConfig =
        &anthropic_config(Some("sk-ant-key"), None, DEFAULT_BASE);
    let mut headers = HeaderMap::new();
    config
        .validate_environment(&mut headers, None, None)
        .unwrap();
    assert_eq!(
        config.get_complete_url(None, "skills", None).unwrap(),
        "https://api.anthropic.com/v1/skills"
    );
    assert_eq!(
        config.transform_get_skill_request("skill_1", None).unwrap(),
        "https://api.anthropic.com/v1/skills/skill_1"
    );
    config
        .transform_create_skill_response(
            br#"{"id":"s","created_at":"a","source":"custom","updated_at":"b"}"#,
        )
        .unwrap();
    config
        .transform_list_skills_response(br#"{"data":[]}"#)
        .unwrap();
    config
        .transform_delete_skill_response(br#"{"id":"s"}"#)
        .unwrap();
    let error = config.get_error_class("boom", 502, &BTreeMap::new());
    assert_eq!(error.status_code, 502);
}
