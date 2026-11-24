use std::collections::{BTreeMap, BTreeSet};

use crate::openapi_tools::tool::DEFAULT_AUTH_HEADER;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct OpenApiCallContext {
    pub auth_header: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub resolved_auth_headers: BTreeMap<String, String>,
}

// LiteLLM's _merge_openapi_tool_request_headers: extra, static, BYOK Authorization, resolved credential
#[must_use]
pub fn merge_request_headers(
    static_headers: &BTreeMap<String, String>,
    context: &OpenApiCallContext,
) -> BTreeMap<String, String> {
    let static_lowered: BTreeSet<String> = static_headers
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut effective: BTreeMap<String, String> = context
        .extra_headers
        .iter()
        .filter(|(name, _)| !static_lowered.contains(&name.to_ascii_lowercase()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    effective.extend(
        static_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    if let Some(auth) = context
        .auth_header
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        effective.retain(|name, _| !name.eq_ignore_ascii_case(DEFAULT_AUTH_HEADER));
        effective.insert(DEFAULT_AUTH_HEADER.to_owned(), auth.to_owned());
    }
    for (name, value) in &context.resolved_auth_headers {
        effective.retain(|existing, _| !existing.eq_ignore_ascii_case(name));
        effective.insert(name.clone(), value.clone());
    }
    effective
}
