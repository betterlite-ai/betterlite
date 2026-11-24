use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_dcr::oauth_error::{OauthError, OauthErrorResponse as _};
use betterlite_mcp::gateway_dcr::registration::{
    RegisteredClient, Registration, register_aggregate_client,
};
use betterlite_mcp::redirect_trust::shape::native_callbacks;
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::{RegisterCall, failure};

pub(super) fn reports(
    calls: &[RegisterCall],
    cipher: &DbCipher,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut reports = Vec::with_capacity(calls.len());
    for call in calls {
        reports.push(report(call, cipher)?);
    }
    Ok(reports)
}
fn report(call: &RegisterCall, cipher: &DbCipher) -> Result<Value, Box<HelperError>> {
    let Some(now) = DateTime::<Utc>::from_timestamp(call.now, 0) else {
        return Err(failure("now is not a valid unix timestamp"));
    };
    let configured = call.native_redirect_uris.as_deref().unwrap_or_default();
    let callbacks = native_callbacks(configured);
    let registration = register_aggregate_client(
        &call.body,
        call.token_exchange_available,
        now,
        cipher,
        &callbacks,
    );
    Ok(match registration {
        Registration::Registered(client) => success_report(&client),
        Registration::Refused(error) => error_report(&error),
    })
}
fn success_report(client: &RegisteredClient) -> Value {
    json!({
        "status": 201_u16,
        "error": null,
        "error_description": null,
        "cache_headers": [],
        "client_id_issued_at": client.client_id_issued_at,
        "redirect_uris": client.redirect_uris,
        "token_endpoint_auth_method": client.token_endpoint_auth_method,
        "grant_types": client.grant_types,
        "response_types": client.response_types,
    })
}

fn error_report(error: &OauthError) -> Value {
    let mut headers: Vec<[String; 2]> = error
        .headers()
        .into_iter()
        .map(|(name, value)| [name.to_lowercase(), value.to_owned()])
        .collect();
    headers.sort();
    json!({
        "status": error.status,
        "error": <&'static str>::from(error.error),
        "error_description": error.description,
        "cache_headers": headers,
        "client_id_issued_at": null,
        "redirect_uris": null,
        "token_endpoint_auth_method": null,
        "grant_types": null,
        "response_types": null,
    })
}
