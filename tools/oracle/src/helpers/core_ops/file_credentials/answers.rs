use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_core::api::file_credentials::accounting::{
    add_deployment_model_info, add_internal_model_credentials,
};
use betterlite_core::api::file_credentials::hints::{ModelHints, extract_model_from_sources};
use betterlite_core::api::file_credentials::routing::{
    AuthorizedCredentialsError, RoutingOutcome, RoutingRequest, handle_model_based_routing,
};
use betterlite_core::api::file_credentials::team::{
    apply_team_credentials, get_team_provider_credentials,
};
use betterlite_models::controls::call_params::TRUSTED_CREDENTIALS_KEY;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::catalog::{Scenario, ScriptedCatalog, locked, scenario};
use super::failure;
use super::input::{Call, KeyInput, RouterInput};

// the text-valued view of a credentials mapping, as the Python helper answers it
fn text_map(value: Option<&Json>) -> Option<BTreeMap<String, String>> {
    let Some(Json::Object(fields)) = value else {
        return None;
    };
    let texts = fields
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect();
    Some(texts)
}

fn routed_answer(routed: &RoutingOutcome) -> Value {
    json!({
        "function": "routing",
        "use_model_routing": routed.use_model_routing,
        "model": routed.model,
        "original_file_id": routed.original_file_id,
        "credentials": routed.credentials,
        "error": Value::Null,
    })
}

// a denial would carry B886's status; the scripted grant never answers one
fn denied_answer(denied: AuthorizedCredentialsError<Infallible, Infallible>) -> Value {
    let (status, detail) = match denied {
        AuthorizedCredentialsError::Unauthorized(never) => match never {},
        AuthorizedCredentialsError::Credentials(lookup) => {
            (lookup.status(), lookup.detail_error().map(str::to_owned))
        }
    };
    json!({
        "function": "routing",
        "use_model_routing": false,
        "model": Value::Null,
        "original_file_id": Value::Null,
        "credentials": Value::Null,
        "error": json!({"status": status, "detail": detail}),
    })
}

fn extract_answer(file_id: &str, input_hints: &ModelHints) -> HelperResult {
    let (embedded, param) = extract_model_from_sources(file_id, input_hints);
    Ok(json!({"function": "extract", "embedded": embedded, "param": param}))
}

fn routing_answer(
    file_id: &str,
    input_hints: &ModelHints,
    check_file_id_encoding: bool,
    router: Option<&RouterInput>,
) -> HelperResult {
    let built = scenario(&KeyInput::default(), router)?;
    let request = RoutingRequest {
        file_id,
        hints: input_hints,
        check_file_id_encoding,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(error.to_string()))?;
    let outcome = runtime.block_on(handle_model_based_routing(
        built.catalog.as_ref(),
        &built.grants,
        &request,
        &built.identity,
    ));
    match outcome {
        Ok(routed) => Ok(routed_answer(&routed)),
        Err(denied) => Ok(denied_answer(denied)),
    }
}

fn team_answer(provider: &str, key: &KeyInput, router: Option<&RouterInput>) -> HelperResult {
    let built = scenario(key, router)?;
    let found = get_team_provider_credentials(
        built.catalog.as_ref(),
        &built.identity,
        provider,
        &built.expansion.context(),
    )
    .map_err(|error| failure(error.to_string()))?;
    let lookups = match built.catalog.as_ref() {
        Some(catalog) => locked(&catalog.lookups).clone(),
        None => Vec::new(),
    };
    Ok(json!({"function": "team_credentials", "credentials": found, "lookups": lookups}))
}

fn apply_answer(
    provider: &str,
    key: &KeyInput,
    data: &JsonObject,
    router: Option<&RouterInput>,
) -> HelperResult {
    let built = scenario(key, router)?;
    let merged = merged_data(&built, provider, data)?;
    Ok(json!({"function": "apply_team_credentials", "data": merged}))
}

fn merged_data(
    built: &Scenario<'_>,
    provider: &str,
    data: &JsonObject,
) -> Result<JsonObject, Box<HelperError>> {
    let mut merged = data.clone();
    apply_team_credentials(
        &mut merged,
        built.catalog.as_ref(),
        &built.identity,
        provider,
        &built.expansion.context(),
    )
    .map_err(|error| failure(error.to_string()))?;
    Ok(merged)
}

fn internal_answer(
    model_id: Option<&str>,
    data: &JsonObject,
    router: &RouterInput,
) -> HelperResult {
    let catalog = ScriptedCatalog::new(router);
    // a caller-supplied mapping under the internal key, which the Python side reports on a miss
    let caller_snapshot = text_map(data.get(TRUSTED_CREDENTIALS_KEY));
    let mut merged = data.clone();
    let installed = add_internal_model_credentials(&mut merged, Some(&catalog), model_id);
    merged.shift_remove(TRUSTED_CREDENTIALS_KEY);
    let installed_snapshot = installed.map(|trusted| {
        trusted
            .values
            .iter()
            .map(|(name, secret)| (name.clone(), secret.expose_secret().to_owned()))
            .collect::<BTreeMap<_, _>>()
    });
    let snapshot = installed_snapshot.or(caller_snapshot);
    // this side answers the stored mapping out of band, so the snapshot says the key would be there
    let internal_key = snapshot.is_some();
    Ok(json!({
        "function": "internal_credentials",
        "data": merged,
        "internal_key": internal_key,
        "snapshot": snapshot,
    }))
}

fn info_answer(model_id: &str, data: &JsonObject, router: Option<&RouterInput>) -> HelperResult {
    let catalog = router.map(ScriptedCatalog::new);
    let mut stamped = data.clone();
    add_deployment_model_info(&mut stamped, catalog.as_ref(), model_id)
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!({"function": "deployment_info", "data": stamped}))
}

pub(crate) fn answer(call: &Call) -> HelperResult {
    match call {
        Call::Extract { file_id, hints } => extract_answer(file_id, hints),
        Call::Routing {
            file_id,
            hints,
            check_file_id_encoding,
            router,
        } => routing_answer(file_id, hints, *check_file_id_encoding, router.as_ref()),
        Call::TeamCredentials {
            provider,
            key,
            router,
        } => team_answer(provider, key, router.as_ref()),
        Call::ApplyTeamCredentials {
            provider,
            key,
            data,
            router,
        } => apply_answer(provider, key, data, router.as_ref()),
        Call::InternalCredentials {
            model_id,
            data,
            router,
        } => internal_answer(model_id.as_deref(), data, router),
        Call::DeploymentInfo {
            model_id,
            data,
            router,
        } => info_answer(model_id, data, router.as_ref()),
    }
}
