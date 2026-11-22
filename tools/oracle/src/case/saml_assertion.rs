use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use super::browser_flow::supported_capture_name;

pub(crate) const MAX_SUBJECT_BYTES: usize = 512;
pub(crate) const MAX_TEXT_BYTES: usize = 1024;
pub(crate) const MAX_DESTINATION_BYTES: usize = 2048;
pub(crate) const MAX_ASSERTION_ID_BYTES: usize = 256;
pub(crate) const MAX_ATTRIBUTES: usize = 32;
pub(crate) const MAX_ATTRIBUTE_VALUES: usize = 128;
pub(crate) const MAX_ATTRIBUTE_BYTES: usize = 4096;
pub(crate) const MAX_RELAY_STATE_BYTES: usize = 1024;
pub(crate) const MAX_OFFSET_SECONDS: i64 = 86_400;

const DEFAULT_NOT_BEFORE_SECONDS: i64 = -60;
const DEFAULT_NOT_AFTER_SECONDS: i64 = 300;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SamlAssertion {
    pub(crate) subject: String,
    pub(crate) attributes: BTreeMap<String, Vec<String>>,
    pub(crate) audience: String,
    pub(crate) destination: String,
    pub(crate) request_id_capture: String,
    pub(crate) assertion_id: String,
    #[serde(default = "default_not_before_seconds")]
    pub(crate) not_before_offset_seconds: i64,
    #[serde(default = "default_not_after_seconds")]
    pub(crate) not_on_or_after_offset_seconds: i64,
    #[serde(default)]
    pub(crate) relay_state: Option<String>,
    #[serde(default)]
    pub(crate) mutation: Option<Mutation>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mutation {
    Unsigned,
    WrongKey,
    Expired,
    WrongAudience,
    WrongDestination,
    SubjectConfirmationOnly,
}

const fn default_not_before_seconds() -> i64 {
    DEFAULT_NOT_BEFORE_SECONDS
}

const fn default_not_after_seconds() -> i64 {
    DEFAULT_NOT_AFTER_SECONDS
}

impl SamlAssertion {
    pub(crate) fn validate(&self) -> Result<(), String> {
        check_text("subject", &self.subject, MAX_SUBJECT_BYTES)?;
        check_text("audience", &self.audience, MAX_TEXT_BYTES)?;
        check_text("destination", &self.destination, MAX_DESTINATION_BYTES)?;
        check_destination(&self.destination)?;
        check_assertion_id(&self.assertion_id)?;
        check_capture_name(&self.request_id_capture)?;
        if let Some(relay_state) = &self.relay_state {
            check_length("relay_state", relay_state, MAX_RELAY_STATE_BYTES)?;
        }
        self.validate_attributes()?;
        self.validate_offsets()
    }

    fn validate_attributes(&self) -> Result<(), String> {
        if self.attributes.is_empty() {
            return Err("attributes cannot be empty".to_owned());
        }
        if self.attributes.len() > MAX_ATTRIBUTES {
            return Err(format!("attributes cannot exceed {MAX_ATTRIBUTES} entries"));
        }
        let mut values = 0_usize;
        for (name, attribute_values) in &self.attributes {
            check_text("attribute name", name, MAX_TEXT_BYTES)?;
            if attribute_values.is_empty() {
                return Err(format!("attribute \"{name}\" has no values"));
            }
            for value in attribute_values {
                check_length("attribute value", value, MAX_ATTRIBUTE_BYTES)?;
                values = values.saturating_add(1);
            }
        }
        if values > MAX_ATTRIBUTE_VALUES {
            return Err(format!(
                "attribute values cannot exceed {MAX_ATTRIBUTE_VALUES}"
            ));
        }
        Ok(())
    }

    fn validate_offsets(&self) -> Result<(), String> {
        if self.mutation == Some(Mutation::Expired) {
            return Ok(());
        }
        let limit = MAX_OFFSET_SECONDS.unsigned_abs();
        if self.not_before_offset_seconds.unsigned_abs() > limit
            || self.not_on_or_after_offset_seconds.unsigned_abs() > limit
        {
            return Err(format!(
                "validity offsets cannot exceed {MAX_OFFSET_SECONDS} seconds"
            ));
        }
        if self.not_before_offset_seconds >= self.not_on_or_after_offset_seconds {
            return Err(
                "not_before_offset_seconds must be before not_on_or_after_offset_seconds"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

fn check_text(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    check_length(field, value, max)
}

fn check_length(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{field} cannot exceed {max} bytes"));
    }
    Ok(())
}

fn check_destination(destination: &str) -> Result<(), String> {
    let Ok(url) = Url::parse(destination) else {
        return Err("destination must be an absolute http or https URL".to_owned());
    };
    if matches!(url.scheme(), "http" | "https") {
        Ok(())
    } else {
        Err("destination must be an absolute http or https URL".to_owned())
    }
}

fn check_assertion_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_ASSERTION_ID_BYTES {
        return Err(format!(
            "assertion_id must be 1 to {MAX_ASSERTION_ID_BYTES} bytes"
        ));
    }
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return Err("assertion_id cannot be empty".to_owned());
    };
    let valid_first = first.is_ascii_alphabetic() || first == '_';
    let valid_rest = characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'));
    if valid_first && valid_rest {
        Ok(())
    } else {
        Err("assertion_id must be a valid XML ID".to_owned())
    }
}

fn check_capture_name(name: &str) -> Result<(), String> {
    if name.is_empty() || !supported_capture_name(name) {
        return Err("request_id_capture has unsupported characters".to_owned());
    }
    Ok(())
}
