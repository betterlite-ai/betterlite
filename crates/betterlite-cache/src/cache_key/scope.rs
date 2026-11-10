use super::request::CacheKeyRequest;
use super::value::field;
use crate::types::semantic::SemanticCacheScope;

pub const TENANT_SCOPE_FIELDS: [&str; 3] = [
    "user_api_key",
    "user_api_key_team_id",
    "user_api_key_org_id",
];
pub const END_USER_SCOPE_FIELD: &str = "user_api_key_end_user_id";
pub const SEMANTIC_EXCLUDED_PARAMS: [&str; 3] = ["input", "messages", "prompt"];

// A field present in several metadata sources takes the first source that holds a non-null value.
pub(crate) fn tenant_scope(request: &CacheKeyRequest, scope: SemanticCacheScope) -> String {
    let litellm_params = request.litellm_params.as_ref();
    let sources = [
        request.metadata.as_ref(),
        request.litellm_metadata.as_ref(),
        litellm_params.and_then(|params| params.metadata.as_ref()),
        litellm_params.and_then(|params| params.litellm_metadata.as_ref()),
    ];
    let end_user = match scope {
        SemanticCacheScope::Key => None,
        SemanticCacheScope::EndUser => Some(END_USER_SCOPE_FIELD),
    };
    let mut rendered = String::new();
    for name in TENANT_SCOPE_FIELDS.into_iter().chain(end_user) {
        let Some(value) = sources.iter().find_map(|source| field(*source, name)) else {
            continue;
        };
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(&value.render());
    }
    rendered
}
