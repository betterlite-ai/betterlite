pub mod auth;
pub mod chat;
pub mod error;
pub mod messages;
pub mod passthrough;

mod tests;

use betterlite_openai::openai_base_config::Object;

use error::MantleSettingsError;

// A JSON field read as text: null is absent, and any other non-string names the field (docs/compatibility.md).
pub(crate) fn field_text(
    params: &Object,
    field: &'static str,
) -> Result<Option<String>, MantleSettingsError> {
    let Some(value) = params.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(str::to_owned)
        .map(Some)
        .ok_or(MantleSettingsError::Field { field })
}
