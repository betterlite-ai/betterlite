use crate::credential_contracts::header_carrier::validate_header_name;
use crate::server_payload::PayloadError;
use crate::server_payload::repr;
use crate::server_payload::request::{NewMCPServerRequest, OrderedMap, UpdateMCPServerRequest};
use crate::server_payload::rules::PayloadRules;

mod tests;

pub struct ToolNameValidation {
    pub is_valid: bool,
    pub warnings: Vec<String>,
}

pub trait NamedPayload {
    fn server_name(&self) -> Option<&str>;
    fn alias(&self) -> Option<&str>;
    fn set_alias(&mut self, alias: Option<String>);
    fn display_names(&self) -> Option<&OrderedMap>;
    fn upstream_token_header(&self) -> Option<&str>;
}

macro_rules! named_payload {
    ($request:ty) => {
        impl NamedPayload for $request {
            fn server_name(&self) -> Option<&str> {
                self.server_name.as_deref()
            }

            fn alias(&self) -> Option<&str> {
                self.alias.as_deref()
            }

            fn set_alias(&mut self, alias: Option<String>) {
                self.alias = alias;
            }

            fn display_names(&self) -> Option<&OrderedMap> {
                self.tool_name_to_display_name.as_ref()
            }

            fn upstream_token_header(&self) -> Option<&str> {
                self.credentials
                    .as_ref()
                    .and_then(|credentials| credentials.upstream_token_header.as_ref())
                    .and_then(|value| value.as_deref())
            }
        }
    };
}

named_payload!(NewMCPServerRequest);
named_payload!(UpdateMCPServerRequest);

pub fn validate_mcp_server_name(name: &str, separator: &str) -> Result<(), PayloadError> {
    if !name.is_empty() && name.contains(separator) {
        return Err(PayloadError::SeparatorInName {
            separator: separator.to_owned(),
            name: name.to_owned(),
        });
    }
    Ok(())
}

#[must_use]
pub fn normalize_server_name(server_name: &str) -> String {
    let mut normalized = String::with_capacity(server_name.len());
    for (index, part) in server_name.split(' ').enumerate() {
        if index > 0 {
            normalized.push('_');
        }
        normalized.push_str(part);
    }
    normalized
}

pub fn validate_tool_display_names(names: &OrderedMap) -> Result<(), PayloadError> {
    for (tool_name, display_name) in names {
        if !display_name.is_empty() && !matches_display_pattern(display_name) {
            return Err(PayloadError::DisplayName {
                display_name: display_name.clone(),
                tool_name: tool_name.clone(),
            });
        }
    }
    Ok(())
}

// Python's `$` also matches before one trailing newline, so a single trailing newline is allowed
fn matches_display_pattern(value: &str) -> bool {
    let trimmed = value.strip_suffix('\n').unwrap_or(value);
    !trimmed.is_empty()
        && trimmed.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

#[must_use]
pub fn validate_tool_name(name: &str) -> ToolNameValidation {
    let length = name.chars().count();
    if length == 0 {
        return invalid(vec!["Tool name cannot be empty".to_owned()]);
    }
    if length > 128 {
        return invalid(vec![format!(
            "Tool name exceeds maximum length of 128 characters (current: {length})"
        )]);
    }
    let mut warnings = Vec::new();
    if name.contains(' ') {
        warnings.push("Tool name contains spaces, which may cause parsing issues".to_owned());
    }
    if name.contains(',') {
        warnings.push("Tool name contains commas, which may cause parsing issues".to_owned());
    }
    if name.starts_with('-') || name.ends_with('-') {
        warnings.push(
            "Tool name starts or ends with a dash, which may cause parsing issues in some contexts"
                .to_owned(),
        );
    }
    if name.starts_with('.') || name.ends_with('.') {
        warnings.push(
            "Tool name starts or ends with a dot, which may cause parsing issues in some contexts"
                .to_owned(),
        );
    }
    if name
        .chars()
        .any(|character| !is_tool_name_character(character))
    {
        warnings.push(invalid_characters_warning(name));
        warnings.push(
            "Allowed characters are: A-Z, a-z, 0-9, underscore (_), dash (-), and dot (.)"
                .to_owned(),
        );
        return ToolNameValidation {
            is_valid: false,
            warnings,
        };
    }
    ToolNameValidation {
        is_valid: true,
        warnings,
    }
}

fn invalid(warnings: Vec<String>) -> ToolNameValidation {
    ToolNameValidation {
        is_valid: false,
        warnings,
    }
}

fn is_tool_name_character(character: char) -> bool {
    if character.is_ascii_alphanumeric() {
        return true;
    }
    matches!(character, '.' | '_' | '-')
}

fn invalid_characters_warning(name: &str) -> String {
    let mut seen = Vec::new();
    for character in name.chars() {
        if !is_tool_name_character(character) && !seen.contains(&character) {
            seen.push(character);
        }
    }
    let rendered = seen
        .iter()
        .map(|character| repr::text(&character.to_string()))
        .collect::<Vec<String>>();
    let joined = rendered.join(", ");
    format!("Tool name contains invalid characters: {joined}")
}

// ast-grep-ignore: no-long-names
pub fn validate_and_normalize_payload(
    payload: &mut impl NamedPayload,
    rules: &PayloadRules,
) -> Result<(), PayloadError> {
    if let Some(name) = payload.server_name().filter(|name| !name.is_empty()) {
        validate_mcp_server_name(name, &rules.prefix_separator)?;
    }
    if let Some(alias) = payload.alias().filter(|alias| !alias.is_empty()) {
        validate_mcp_server_name(alias, &rules.prefix_separator)?;
    }
    if let Some(names) = payload.display_names() {
        validate_tool_display_names(names)?;
    }
    let alias = payload.alias().filter(|alias| !alias.is_empty());
    let server_name = payload.server_name().filter(|name| !name.is_empty());
    let normalized = match (alias, server_name) {
        (None, Some(name)) => Some(normalize_server_name(name)),
        (Some(alias), _) => Some(normalize_server_name(alias)),
        (None, None) => None,
    };
    payload.set_alias(normalized);
    if let Some(name) = payload.server_name().filter(|name| !name.is_empty()) {
        validate_prefix("server_name", name)?;
    }
    if let Some(alias) = payload.alias().filter(|alias| !alias.is_empty()) {
        validate_prefix("alias", alias)?;
    }
    check_upstream_token_header(payload)
}

fn validate_prefix(field: &'static str, value: &str) -> Result<(), PayloadError> {
    let validation = validate_tool_name(value);
    if validation.is_valid {
        return Ok(());
    }
    let warnings = if validation.warnings.is_empty() {
        String::new()
    } else {
        format!("\n{}", validation.warnings.join("\n"))
    };
    Err(PayloadError::ToolPrefix {
        value: value.to_owned(),
        field,
        warnings,
    })
}

fn check_upstream_token_header(payload: &impl NamedPayload) -> Result<(), PayloadError> {
    let Some(raw) = payload.upstream_token_header() else {
        return Ok(());
    };
    if raw.is_empty() || validate_header_name(raw).is_ok() {
        return Ok(());
    }
    Err(PayloadError::UpstreamTokenHeader {
        rendered: repr::text(raw),
    })
}
