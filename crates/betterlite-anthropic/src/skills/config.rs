use std::collections::BTreeMap;

use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::SKILLS_BETA_VERSION;
use super::error::SkillsError;
use super::wire::{
    CreateSkillRequest, DeleteSkillResponse, ListSkillsParams, ListSkillsResponse, Skill,
};
use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings, get_auth_header};

const SKILLS_ENDPOINT: &str = "skills";

pub trait BaseSkillsApiConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: &mut HeaderMap,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<(), SkillsError>;

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        skill_id: Option<&str>,
    ) -> Result<String, SkillsError>;

    fn transform_create_skill_request(&self, request: &CreateSkillRequest) -> CreateSkillRequest;

    // ast-grep-ignore: no-long-names
    fn transform_create_skill_response(&self, body: &[u8]) -> Result<Skill, SkillsError>;

    fn transform_list_skills_request(
        &self,
        params: &ListSkillsParams,
        api_base: Option<&str>,
    ) -> Result<(String, Vec<(String, String)>), SkillsError>;

    fn transform_list_skills_response(
        &self,
        body: &[u8],
    ) -> Result<ListSkillsResponse, SkillsError>;

    fn transform_get_skill_request(
        &self,
        skill_id: &str,
        api_base: Option<&str>,
    ) -> Result<String, SkillsError>;

    fn transform_get_skill_response(&self, body: &[u8]) -> Result<Skill, SkillsError>;

    fn transform_delete_skill_request(
        &self,
        skill_id: &str,
        api_base: Option<&str>,
    ) -> Result<String, SkillsError>;

    // ast-grep-ignore: no-long-names
    fn transform_delete_skill_response(
        &self,
        body: &[u8],
    ) -> Result<DeleteSkillResponse, SkillsError>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &BTreeMap<String, String>,
    ) -> ProviderError {
        let pairs: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: (!pairs.is_empty()).then_some(pairs),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicSkillsConfig {
    pub settings: AnthropicSettings,
}

impl BaseSkillsApiConfig for AnthropicSkillsConfig {
    fn validate_environment(
        &self,
        headers: &mut HeaderMap,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<(), SkillsError> {
        let auth = get_auth_header(api_key, api_base, false, &self.settings)
            .ok_or(SkillsError::MissingCredential)?;
        let mut value = HeaderValue::from_str(auth.value.expose_secret())
            .map_err(|_invalid| SkillsError::InvalidHeader { name: auth.name })?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static(auth.name), value);
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        insert_beta_header(headers);
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        Ok(())
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        skill_id: Option<&str>,
    ) -> Result<String, SkillsError> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.settings.api_base);
        let Some(target) = skill_id.filter(|id| !id.is_empty()) else {
            return Ok(format!("{base}/v1/{endpoint}"));
        };
        let encoded = encode_segment(target, "skill_id")?;
        Ok(format!("{base}/v1/skills/{encoded}"))
    }

    fn transform_create_skill_request(&self, request: &CreateSkillRequest) -> CreateSkillRequest {
        let mut extra: IndexMap<String, Json> = IndexMap::new();
        for (name, value) in &request.extra {
            if !matches!(value, Json::Null) {
                extra.insert(name.clone(), value.clone());
            }
        }
        CreateSkillRequest {
            display_title: request.display_title.clone(),
            files: request.files.clone(),
            extra,
        }
    }

    // ast-grep-ignore: no-long-names
    fn transform_create_skill_response(&self, body: &[u8]) -> Result<Skill, SkillsError> {
        serde_json::from_slice(body).map_err(|source| SkillsError::Decode { source })
    }

    fn transform_list_skills_request(
        &self,
        params: &ListSkillsParams,
        api_base: Option<&str>,
    ) -> Result<(String, Vec<(String, String)>), SkillsError> {
        let url = self.get_complete_url(api_base, SKILLS_ENDPOINT, None)?;
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(limit) = params.limit.filter(|limit| *limit != 0) {
            query.push(("limit".to_owned(), limit.to_string()));
        }
        if let Some(page) = params.page.as_deref().filter(|page| !page.is_empty()) {
            query.push(("page".to_owned(), page.to_owned()));
        }
        if let Some(source) = params.source.as_deref().filter(|source| !source.is_empty()) {
            query.push(("source".to_owned(), source.to_owned()));
        }
        Ok((url, query))
    }

    fn transform_list_skills_response(
        &self,
        body: &[u8],
    ) -> Result<ListSkillsResponse, SkillsError> {
        serde_json::from_slice(body).map_err(|source| SkillsError::Decode { source })
    }

    fn transform_get_skill_request(
        &self,
        skill_id: &str,
        api_base: Option<&str>,
    ) -> Result<String, SkillsError> {
        self.get_complete_url(api_base, SKILLS_ENDPOINT, Some(skill_id))
    }

    fn transform_get_skill_response(&self, body: &[u8]) -> Result<Skill, SkillsError> {
        serde_json::from_slice(body).map_err(|source| SkillsError::Decode { source })
    }

    fn transform_delete_skill_request(
        &self,
        skill_id: &str,
        api_base: Option<&str>,
    ) -> Result<String, SkillsError> {
        self.get_complete_url(api_base, SKILLS_ENDPOINT, Some(skill_id))
    }

    // ast-grep-ignore: no-long-names
    fn transform_delete_skill_response(
        &self,
        body: &[u8],
    ) -> Result<DeleteSkillResponse, SkillsError> {
        serde_json::from_slice(body).map_err(|source| SkillsError::Decode { source })
    }
}

fn insert_beta_header(headers: &mut HeaderMap) {
    let name = HeaderName::from_static("anthropic-beta");
    let existing: Vec<HeaderValue> = headers.get_all(&name).iter().cloned().collect();
    if existing.is_empty() {
        headers.insert(name, HeaderValue::from_static(SKILLS_BETA_VERSION));
        return;
    }
    let present = match existing.as_slice() {
        // one header value is the caller's string, and Python tests it with substring containment
        [value] => value
            .to_str()
            .is_ok_and(|text| text.contains(SKILLS_BETA_VERSION)),
        // several header values are the caller's list, where Python tests exact membership
        values => values
            .iter()
            .any(|value| value.to_str().is_ok_and(|text| text == SKILLS_BETA_VERSION)),
    };
    if !present {
        headers.append(name, HeaderValue::from_static(SKILLS_BETA_VERSION));
    }
}
