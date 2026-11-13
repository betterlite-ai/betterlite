use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

use super::error::ToolPermissionError;

mod tests;

// LiteLLM's _ToolPermissionRuleFields: the only keys a config rule may carry
pub const RULE_FIELDS: [&str; 5] = [
    "allowed_param_patterns",
    "decision",
    "id",
    "tool_name",
    "tool_type",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RuleFields")]
pub struct ToolPermissionRule {
    pub id: String,
    pub tool_name: Option<String>,
    pub tool_type: Option<String>,
    pub decision: ToolDecision,
    pub allowed_param_patterns: Option<BTreeMap<String, String>>,
}

// The config-file shape a ToolPermissionRule is built from, validated in ToolPermissionRule's TryFrom
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFields {
    id: String,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_type: Option<String>,
    decision: String,
    #[serde(default)]
    allowed_param_patterns: Option<BTreeMap<String, String>>,
}

impl TryFrom<RuleFields> for ToolPermissionRule {
    type Error = ToolPermissionError;

    fn try_from(fields: RuleFields) -> Result<Self, Self::Error> {
        let tool_name = normalize_target(fields.tool_name);
        let tool_type = normalize_target(fields.tool_type);
        if tool_name.is_none() && tool_type.is_none() {
            return Err(ToolPermissionError::MissingTarget);
        }
        Ok(Self {
            id: fields.id,
            tool_name,
            tool_type,
            decision: parse_decision(&fields.decision)?,
            allowed_param_patterns: fields.allowed_param_patterns,
        })
    }
}

impl ToolPermissionRule {
    pub fn from_fields(
        fields: &BTreeMap<String, GuardrailValue>,
    ) -> Result<Self, ToolPermissionError> {
        for field in fields.keys() {
            if !RULE_FIELDS.contains(&field.as_str()) {
                return Err(ToolPermissionError::UnknownField {
                    field: field.clone(),
                });
            }
        }
        let id =
            text_field(fields, "id")?.ok_or(ToolPermissionError::MissingField { field: "id" })?;
        let decision = text_field(fields, "decision")?
            .ok_or(ToolPermissionError::MissingField { field: "decision" })?;
        Self::try_from(RuleFields {
            id,
            tool_name: text_field(fields, "tool_name")?,
            tool_type: text_field(fields, "tool_type")?,
            decision,
            allowed_param_patterns: patterns_field(fields)?,
        })
    }
}

// LiteLLM's _blank_to_none strips a target and treats an all-whitespace one as absent
fn normalize_target(target: Option<String>) -> Option<String> {
    target
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

// LiteLLM lowercases before its allow/deny check
fn parse_decision(value: &str) -> Result<ToolDecision, ToolPermissionError> {
    if value.eq_ignore_ascii_case("allow") {
        return Ok(ToolDecision::Allow);
    }
    if value.eq_ignore_ascii_case("deny") {
        return Ok(ToolDecision::Deny);
    }
    Err(ToolPermissionError::InvalidDecision {
        value: value.to_owned(),
    })
}

fn text_field(
    fields: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<Option<String>, ToolPermissionError> {
    match fields.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_wrong) => Err(ToolPermissionError::WrongType {
            field: field.to_owned(),
            expected: "a string",
        }),
    }
}

fn patterns_field(
    fields: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<BTreeMap<String, String>>, ToolPermissionError> {
    match fields.get("allowed_param_patterns") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(entries)) => entries
            .iter()
            .map(|(path, value)| match value {
                GuardrailValue::Text(pattern) => Ok((path.clone(), pattern.clone())),
                _wrong => Err(ToolPermissionError::WrongType {
                    field: format!("allowed_param_patterns.{path}"),
                    expected: "a string",
                }),
            })
            .collect::<Result<BTreeMap<String, String>, ToolPermissionError>>()
            .map(Some),
        Some(_wrong) => Err(ToolPermissionError::WrongType {
            field: "allowed_param_patterns".to_owned(),
            expected: "an object of string patterns",
        }),
    }
}
