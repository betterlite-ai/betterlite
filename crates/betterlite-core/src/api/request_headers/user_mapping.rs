use betterlite_config::sections::request_headers::{Settings, UserHeaderMapping, UserHeaderRole};

use crate::api::request_headers::ForwardedHeaders;

#[must_use]
pub fn case_insensitive_header<'headers>(
    headers: &'headers ForwardedHeaders,
    name: &str,
) -> Option<&'headers str> {
    let wanted = name.to_lowercase();
    headers
        .iter()
        .find(|(header, _value)| header.to_lowercase() == wanted)
        .map(|(_header, value)| value.expose())
}

// the deprecated user_header_name: its value becomes the request's end user
#[must_use]
pub fn user_from_headers(headers: &ForwardedHeaders, settings: &Settings) -> Option<String> {
    let header_name = settings
        .user_header_name
        .as_deref()
        .filter(|header_name| !header_name.is_empty())?;
    let user = case_insensitive_header(headers, header_name)?;
    tracing::info!("found user \"{user}\" in header \"{header_name}\"");
    Some(user.to_owned())
}

// the first internal_user mapping names the header whose value replaces the key's user_id
#[must_use]
pub fn internal_user_from_mapping(
    headers: &ForwardedHeaders,
    mappings: &[UserHeaderMapping],
) -> Option<String> {
    let mapping = mappings.iter().find(|mapping| {
        mapping.litellm_user_role == UserHeaderRole::InternalUser && !mapping.header_name.is_empty()
    })?;
    case_insensitive_header(headers, &mapping.header_name)
        .filter(|user_id| !user_id.is_empty())
        .map(str::to_owned)
}
