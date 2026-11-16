use crate::credential_contracts::header_carrier::validate_header_name;
use crate::server_payload::request::{NewMCPServerRequest, OrderedMap, UpdateMCPServerRequest};
use crate::server_payload::{PayloadError, quote};
use betterlite_config::sections::mcp_env_values::PayloadRules;
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
                let credentials = self.credentials.as_ref()?;
                credentials.upstream_token_header.as_ref()?.as_deref()
            }
        }
    };
}
named_payload!(NewMCPServerRequest);
named_payload!(UpdateMCPServerRequest);
pub fn validate_mcp_server_name(name: &str, separator: &str) -> Result<(), PayloadError> {
    if name.is_empty() || !name.contains(separator) {
        return Ok(());
    }
    Err(PayloadError::SeparatorInName {
        separator: separator.to_owned(),
        name: name.to_owned(),
    })
}
pub fn validate_tool_display_names(names: &OrderedMap) -> Result<(), PayloadError> {
    let invalid = names.iter().find(|(_, display_name)| {
        !display_name.is_empty() && !matches_display_pattern(display_name)
    });
    let Some((tool_name, display_name)) = invalid else {
        return Ok(());
    };
    Err(PayloadError::DisplayName {
        display_name: display_name.clone(),
        tool_name: tool_name.clone(),
    })
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
        warnings.push(DASH_WARNING.to_owned());
    }
    if name.starts_with('.') || name.ends_with('.') {
        warnings.push(DOT_WARNING.to_owned());
    }
    let invalid = name.chars().any(|character| {
        !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    });
    if !invalid {
        return ToolNameValidation {
            is_valid: true,
            warnings,
        };
    }
    warnings.push(invalid_characters_warning(name));
    warnings.push(ALLOWED_CHARACTERS_WARNING.to_owned());
    ToolNameValidation {
        is_valid: false,
        warnings,
    }
}
const DASH_WARNING: &str =
    "Tool name starts or ends with a dash, which may cause parsing issues in some contexts";
const DOT_WARNING: &str =
    "Tool name starts or ends with a dot, which may cause parsing issues in some contexts";
const ALLOWED_CHARACTERS_WARNING: &str =
    "Allowed characters are: A-Z, a-z, 0-9, underscore (_), dash (-), and dot (.)";
const fn invalid(warnings: Vec<String>) -> ToolNameValidation {
    ToolNameValidation {
        is_valid: false,
        warnings,
    }
}
fn invalid_characters_warning(name: &str) -> String {
    let mut seen = Vec::new();
    for character in name.chars() {
        let allowed = character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-');
        if !allowed && !seen.contains(&character) {
            seen.push(character);
        }
    }
    let rendered: Vec<String> = seen
        .iter()
        .map(|character| quote(&character.to_string()))
        .collect();
    format!(
        "Tool name contains invalid characters: {}",
        rendered.join(", ")
    )
}
pub fn validate_and_normalize_payload<P>(
    payload: &mut P,
    rules: &PayloadRules,
) -> Result<(), PayloadError>
where
    P: NamedPayload,
{
    let raw_alias = payload.alias();
    let server_name = payload.server_name().filter(|name| !name.is_empty());
    let alias = raw_alias.filter(|text| !text.is_empty());
    for name in [server_name, alias].into_iter().flatten() {
        validate_mcp_server_name(name, &rules.prefix_separator)?;
    }
    if let Some(names) = payload.display_names() {
        validate_tool_display_names(names)?;
    }
    let normalized = match (alias, server_name) {
        (Some(chosen), _) | (None, Some(chosen)) => Some(chosen.replace(' ', "_")),
        (None, None) => raw_alias.map(str::to_owned),
    };
    payload.set_alias(normalized);
    for (field, candidate) in [
        ("server_name", payload.server_name()),
        ("alias", payload.alias()),
    ] {
        if let Some(value) = candidate.filter(|text| !text.is_empty()) {
            validate_prefix(field, value)?;
        }
    }
    check_upstream_token_header(payload.upstream_token_header())
}
fn validate_prefix(field: &'static str, value: &str) -> Result<(), PayloadError> {
    let validation = validate_tool_name(value);
    if validation.is_valid {
        return Ok(());
    }
    Err(PayloadError::ToolPrefix {
        value: value.to_owned(),
        field,
        warnings: format!("\n{}", validation.warnings.join("\n")),
    })
}
fn check_upstream_token_header(header: Option<&str>) -> Result<(), PayloadError> {
    let Some(raw) = header else {
        return Ok(());
    };
    if raw.is_empty() || validate_header_name(raw).is_ok() {
        return Ok(());
    }
    Err(PayloadError::UpstreamTokenHeader {
        rendered: quote(raw),
    })
}
