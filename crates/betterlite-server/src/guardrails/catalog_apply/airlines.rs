use axum::Json;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_guardrails::content_filter::competitor_intent_airline::MAJOR_AIRLINES_JSON;
use serde::{Deserialize, Serialize};

use crate::auth::auth_identity::AuthContext;
use crate::error_body::json_error;
use crate::tracing_routes::router::unauthenticated;

pub(super) const AIRLINES_PATH: &str = "/guardrails/ui/major_airlines";

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub(super) struct MajorAirline {
    pub(super) id: String,
    #[serde(rename = "match")]
    pub(super) matches: String,
    pub(super) tags: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AirlinesBody {
    airlines: Vec<MajorAirline>,
}

pub(super) async fn major_airlines(caller: Option<Extension<AuthContext>>) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let Ok(airlines) = serde_json::from_str::<Vec<MajorAirline>>(MAJOR_AIRLINES_JSON) else {
        tracing::error!("apply_guardrail: major_airlines.json does not parse");
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Error reading major_airlines.json: the bundled file does not parse",
            "internal_server_error",
            "500",
        );
    };
    (StatusCode::OK, Json(AirlinesBody { airlines })).into_response()
}
