mod tests;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::api::request_form::values::FormParts;
use betterlite_mcp::byok_token_exchange::exchange::exchange_authorization_code;
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup, ByokCredentialSink};
use betterlite_mcp::byok_token_exchange::{TokenError, TokenForm};
use betterlite_mcp::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;

use crate::api::request_form::{
    DetailBody, URLENCODED_FORM_CONTENT_TYPES, ValidationBody, last_value, read_form,
    validation_body,
};
use crate::routes::mcp_byok_oauth::{ByokOauthRoutes, RequestBaseUrl};

const REQUIRED_FIELDS: [&str; 3] = ["grant_type", "code", "code_verifier"];

pub(crate) async fn token_endpoint<C, S, B>(
    State(state): State<Arc<ByokOauthRoutes<C, S, B>>>,
    request: Request,
) -> Response
where
    C: AuthCodeLookup + 'static,
    S: ByokCredentialSink + 'static,
    B: RequestBaseUrl + 'static,
{
    let form_parts = match read_form(request, URLENCODED_FORM_CONTENT_TYPES).await {
        Ok(form_parts) => form_parts,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(DetailBody {
                    detail: error.detail(),
                }),
            )
                .into_response();
        }
    };
    let form = match required_form(&form_parts) {
        Ok(form) => form,
        Err(rejection) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(rejection)).into_response();
        }
    };
    let Ok(clock) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        let failure = TokenError::ServerError.response();
        return (failure.status, TOKEN_NO_CACHE_HEADERS, Json(failure.body)).into_response();
    };
    let result = exchange_authorization_code(
        &form,
        &state.codes,
        state.credentials.as_ref(),
        state.master_key.as_ref(),
        clock.as_secs_f64(),
    )
    .await;
    match result {
        Ok(grant) => (StatusCode::OK, TOKEN_NO_CACHE_HEADERS, Json(grant)).into_response(),
        Err(error) => {
            let failure = error.response();
            (failure.status, TOKEN_NO_CACHE_HEADERS, Json(failure.body)).into_response()
        }
    }
}

fn required_form(parts: &FormParts) -> Result<TokenForm, ValidationBody> {
    let missing: Vec<&'static str> = REQUIRED_FIELDS
        .iter()
        .copied()
        .filter(|field| last_value(parts, field).is_none_or(str::is_empty))
        .collect();
    if !missing.is_empty() {
        return Err(validation_body(&missing));
    }
    let (Some(grant_type), Some(code), Some(code_verifier)) = (
        last_value(parts, "grant_type"),
        last_value(parts, "code"),
        last_value(parts, "code_verifier"),
    ) else {
        return Err(validation_body(&REQUIRED_FIELDS));
    };
    Ok(TokenForm {
        grant_type: grant_type.to_owned(),
        code: code.to_owned(),
        redirect_uri: last_value(parts, "redirect_uri")
            .unwrap_or_default()
            .to_owned(),
        code_verifier: code_verifier.to_owned(),
        client_id: last_value(parts, "client_id")
            .unwrap_or_default()
            .to_owned(),
    })
}
