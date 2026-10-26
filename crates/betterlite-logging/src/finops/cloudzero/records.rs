use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CzbEntityType {
    Team,
}

impl CzbEntityType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Team => "team",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CzrnError {
    Invalid(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageRecord {
    pub date: Option<String>,
    pub custom_llm_provider: String,
    pub team_id: String,
    pub team_alias: Option<String>,
    pub model: String,
    pub model_group: String,
    pub api_key: String,
    pub api_key_alias: Option<String>,
    pub user_email: Option<String>,
    pub organization_alias: Option<String>,
    pub project_alias: Option<String>,
    pub user_alias: Option<String>,
    pub api_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub spend: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CbfValue {
    Text(String),
    Integer(i64),
    Money(Decimal),
    DateTime(DateTime<Utc>),
    None,
}

pub type CbfRecord = BTreeMap<String, CbfValue>;
pub type CbfDataFrame = Vec<CbfRecord>;

#[derive(Debug, Clone, Copy)]
pub struct CzrnComponents<'components> {
    pub service_type: &'components str,
    pub provider: &'components str,
    pub region: &'components str,
    pub owner_account_id: &'components str,
    pub resource_type: &'components str,
    pub cloud_local_id: &'components str,
}

#[derive(Debug, Clone, Default)]
pub struct CzrnGenerator;

impl CzrnGenerator {
    pub fn create_from_litellm_data(&self, row: &UsageRecord) -> Result<String, CzrnError> {
        let provider = Self::normalize_provider(&row.custom_llm_provider);
        let owner_account_id = Self::normalize_component(&row.team_id, false);
        self.create_from_components(CzrnComponents {
            service_type: "litellm",
            provider: &provider,
            region: "cross-region",
            owner_account_id: &owner_account_id,
            resource_type: "llm-usage",
            cloud_local_id: &row.model,
        })
    }

    pub fn create_from_components(
        &self,
        components: CzrnComponents<'_>,
    ) -> Result<String, CzrnError> {
        let czrn = format!(
            "czrn:{}:{}:{}:{}:{}:{}",
            Self::normalize_component(components.service_type, true),
            Self::normalize_component(components.provider, false),
            Self::normalize_component(components.region, false),
            Self::normalize_component(components.owner_account_id, false),
            Self::normalize_component(components.resource_type, false),
            components.cloud_local_id
        );
        if self.is_valid(&czrn) {
            Ok(czrn)
        } else {
            Err(CzrnError::Invalid(czrn))
        }
    }

    #[must_use]
    pub fn is_valid(&self, value: &str) -> bool {
        let mut parts = value.splitn(7, ':');
        parts.next() == Some("czrn")
            && parts
                .next()
                .is_some_and(|part| valid_component(part, false))
            && parts.next().is_some_and(|part| valid_component(part, true))
            && parts
                .next()
                .is_some_and(|part| valid_component(part, false))
            && parts
                .next()
                .is_some_and(|part| valid_component(part, false))
            && parts
                .next()
                .is_some_and(|part| valid_component(part, false))
            && parts.next().is_some_and(|part| !part.is_empty())
    }

    pub fn extract_components(&self, value: &str) -> Result<[String; 6], CzrnError> {
        if !self.is_valid(value) {
            return Err(CzrnError::Invalid(value.to_owned()));
        }
        let mut parts = value.splitn(7, ':');
        let _prefix = parts.next();
        let mut components = [
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        for component in &mut components {
            if let Some(component_value) = parts.next() {
                component_value.clone_into(component);
            }
        }
        Ok(components)
    }

    fn normalize_provider(provider: &str) -> String {
        let normalized = provider.to_lowercase().replace('_', "-");
        match normalized.as_str() {
            "azure" => "azure".to_owned(),
            "anthropic" => "anthropic".to_owned(),
            "bedrock" => "aws".to_owned(),
            "vertex-ai" => "vertex-ai".to_owned(),
            "gemini" => "google".to_owned(),
            "cohere" => "cohere".to_owned(),
            "huggingface" => "huggingface".to_owned(),
            "replicate" => "replicate".to_owned(),
            "together-ai" => "together-ai".to_owned(),
            _ => normalized,
        }
    }

    fn normalize_component(component: &str, allow_uppercase: bool) -> String {
        let value = if allow_uppercase {
            component.to_owned()
        } else {
            component.to_lowercase()
        };
        let mut result = String::new();
        for character in value.chars() {
            let valid = character.is_ascii_alphanumeric() || character == '-';
            if valid || !result.ends_with('-') {
                result.push(component_character(character, valid));
            }
        }
        let trimmed = result.trim_matches('-');
        if trimmed.is_empty() {
            "unknown".to_owned()
        } else {
            trimmed.to_owned()
        }
    }
}

const fn component_character(character: char, valid: bool) -> char {
    if valid { character } else { '-' }
}

fn valid_component(value: &str, uppercase: bool) -> bool {
    if value.is_empty() {
        return false;
    }
    value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || (uppercase && character.is_ascii_uppercase())
    })
}

pub mod cbf;
