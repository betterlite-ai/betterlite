use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::saml_provider::error::{
    NO_USABLE_IDP_DETAIL, SamlProviderError, UNAVAILABLE_DETAIL, UNCONFIGURED_DETAIL,
};
use serde::Serialize;

#[derive(Serialize)]
struct DetailBody<'detail> {
    detail: &'detail str,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

// an HTTPException answers its detail, and any other exception the unhandled handler's generic body
#[must_use]
pub fn provider_error_response(error: &SamlProviderError) -> Response {
    let (status, detail) = match error {
        SamlProviderError::InvalidSettings(errors) => {
            (StatusCode::INTERNAL_SERVER_ERROR, errors.to_string())
        }
        SamlProviderError::InvalidMetadata(errors) => {
            (StatusCode::INTERNAL_SERVER_ERROR, errors.to_string())
        }
        SamlProviderError::Unavailable => {
            (StatusCode::NOT_IMPLEMENTED, UNAVAILABLE_DETAIL.to_owned())
        }
        SamlProviderError::Unconfigured => {
            (StatusCode::NOT_IMPLEMENTED, UNCONFIGURED_DETAIL.to_owned())
        }
        SamlProviderError::NoUsableIdp => {
            (StatusCode::BAD_GATEWAY, NO_USABLE_IDP_DETAIL.to_owned())
        }
        SamlProviderError::InvalidMetadataSettings(_)
        | SamlProviderError::Fetch(_)
        | SamlProviderError::Parse(_)
        | SamlProviderError::InvalidRedirectUrl
        | SamlProviderError::AuthnRequest(_)
        | SamlProviderError::ClockOutOfRange => return internal_error(),
    };
    (status, Json(DetailBody { detail: &detail })).into_response()
}

#[must_use]
pub fn internal_error() -> Response {
    let body = InternalBody {
        error: InternalMessage {
            message: "Internal server error",
            error_type: "internal_server_error",
        },
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}
