use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// What decides whether two deployments decrypt each other's reasoning: their effective api_base and api_key.
pub type Boundary = (String, String);

const CREDENTIAL_NAME: &str = "litellm_credential_name";

// _encryption_boundary_key: a named credential's field wins by presence, and a missing or empty base or key has no boundary.
#[must_use]
pub fn encryption_boundary(
    params: &JsonObject,
    credentials: &CredentialSnapshot,
) -> Option<Boundary> {
    let credential = credential_values(params, credentials);
    let api_base = effective(credential.as_ref(), "api_base", text(params, "api_base"));
    let api_key = effective(credential.as_ref(), "api_key", text(params, "api_key"));
    Some((api_base?, api_key?))
}

// CredentialAccessor.get_credential_values: only a non-empty credential name reads the snapshot, and a miss is empty.
fn credential_values(params: &JsonObject, credentials: &CredentialSnapshot) -> Option<JsonObject> {
    match params.get(CREDENTIAL_NAME) {
        Some(Json::Text(name)) if !name.is_empty() => Some(credentials.values(name)),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        )
        | None => None,
    }
}

// The credential field replaces the deployment's when the credential names it, even with a null value.
fn effective(
    credential: Option<&JsonObject>,
    field: &str,
    fallback: Option<&str>,
) -> Option<String> {
    match credential.and_then(|values| values.get(field)) {
        Some(value) => text_value(value),
        None => fallback.map(str::to_owned),
    }
}

fn text<'params>(params: &'params JsonObject, field: &str) -> Option<&'params str> {
    match params.get(field) {
        Some(Json::Text(value)) if !value.is_empty() => Some(value),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        )
        | None => None,
    }
}

fn text_value(value: &Json) -> Option<String> {
    match value {
        Json::Text(value) if !value.is_empty() => Some(value.clone()),
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_)
        | Json::Text(_) => None,
    }
}
