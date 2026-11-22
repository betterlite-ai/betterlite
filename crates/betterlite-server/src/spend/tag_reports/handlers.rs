use axum::Json;
use axum::extract::{Extension, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;

use crate::auth::auth_identity::AuthContext;
use crate::spend::visibility::roles::is_admin_view_safe;
use crate::team_access::caller::TeamCaller;
use crate::tracing_routes::router::unauthenticated;

use super::aggregation::tag_spend_body;
use super::caller::team_caller;
use super::query::{decode_tag_report_query, selected_tags, selects_every_tag};
use super::responses::{self, NO_DATABASE};
use super::rows::{DailyTagSpend, KeyAliasedUser, TagNamesBody};
use super::store::{TagReportError, TagReportHandle, TagReportStore};

const SPEND_TAGS_FAILED: &str = "/spend/tags Error";
const TAG_NAMES_FAILED: &str = "/spend/all_tag_names Error";
const FORBIDDEN_USER: &str = "Not authorized to view spend for another user.";

pub(super) async fn spend_keys(
    State(store): State<TagReportHandle>,
    identity: Option<Extension<AuthContext>>,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    let Some(store) = store.as_deref() else {
        return responses::detail_error(StatusCode::BAD_REQUEST, NO_DATABASE);
    };
    let caller = match team_caller(&context.principal) {
        Ok(caller) => caller,
        Err(error) => {
            return responses::detail_error(StatusCode::BAD_REQUEST, &error.to_string());
        }
    };
    if is_admin_view_safe(&caller) {
        return key_rows(store.keys().await);
    }
    match caller.user_id.as_deref().filter(|id| !id.is_empty()) {
        Some(user_id) => key_rows(store.keys_for_user(user_id).await),
        None => Json(Vec::<LiteLLMVerificationToken>::new()).into_response(),
    }
}

fn key_rows(result: Result<Vec<LiteLLMVerificationToken>, TagReportError>) -> Response {
    match result {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => responses::detail_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

pub(super) async fn spend_users(
    State(store): State<TagReportHandle>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let Some(Extension(context)) = identity else {
        return unauthenticated();
    };
    let Some(store) = store.as_deref() else {
        return responses::detail_error(StatusCode::BAD_REQUEST, NO_DATABASE);
    };
    let caller = match team_caller(&context.principal) {
        Ok(caller) => caller,
        Err(error) => {
            return responses::detail_error(StatusCode::BAD_REQUEST, &error.to_string());
        }
    };
    let decoded = decode_tag_report_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    let requested = decoded.user_id;
    if !is_admin_view_safe(&caller) {
        return scoped_user_rows(store, &caller, requested.as_deref()).await;
    }
    match requested {
        Some(user_id) => single_user(store.user(&user_id).await),
        None => all_user_rows(store.users().await),
    }
}

// spend_user_fn: without a caller id the request has no scope, before the query parameter is even compared
async fn scoped_user_rows(
    store: &dyn TagReportStore,
    caller: &TeamCaller,
    requested: Option<&str>,
) -> Response {
    let Some(caller_id) = caller.user_id.as_deref().filter(|id| !id.is_empty()) else {
        return Json(Vec::<LiteLLMUserTable>::new()).into_response();
    };
    if requested.is_some_and(|user_id| user_id != caller_id) {
        return responses::detail_error(StatusCode::FORBIDDEN, FORBIDDEN_USER);
    }
    single_user(store.user(caller_id).await)
}

fn single_user(result: Result<Option<LiteLLMUserTable>, TagReportError>) -> Response {
    match result {
        Ok(found) => Json(vec![found]).into_response(),
        Err(error) => responses::detail_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

fn all_user_rows(result: Result<Vec<KeyAliasedUser>, TagReportError>) -> Response {
    match result {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => responses::detail_error(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

pub(super) async fn spend_tags(
    State(store): State<TagReportHandle>,
    identity: Option<Extension<AuthContext>>,
) -> Response {
    if identity.is_none() {
        return unauthenticated();
    }
    let Some(store) = store.as_deref() else {
        return responses::wrapped_error(format!("{SPEND_TAGS_FAILED}{NO_DATABASE}"));
    };
    match store.raw_tag_spend().await {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => tag_failure(SPEND_TAGS_FAILED, &error),
    }
}

pub(super) async fn all_tag_names(
    State(store): State<TagReportHandle>,
    identity: Option<Extension<AuthContext>>,
) -> Response {
    if identity.is_none() {
        return unauthenticated();
    }
    let Some(store) = store.as_deref() else {
        return responses::wrapped_error(format!("{TAG_NAMES_FAILED}{NO_DATABASE}"));
    };
    match store.tag_names().await {
        Ok(Some(tag_names)) => Json(TagNamesBody { tag_names }).into_response(),
        Ok(None) => Json(Vec::<Option<String>>::new()).into_response(),
        Err(error) => tag_failure(TAG_NAMES_FAILED, &error),
    }
}

pub(super) async fn global_spend_tags(
    State(store): State<TagReportHandle>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if identity.is_none() {
        return unauthenticated();
    }
    let Some(store) = store.as_deref() else {
        return responses::wrapped_error(format!("{SPEND_TAGS_FAILED}{NO_DATABASE}"));
    };
    let decoded = decode_tag_report_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    let (Some(start_date), Some(end_date)) =
        (decoded.start_date.as_deref(), decoded.end_date.as_deref())
    else {
        return responses::missing_dates();
    };
    let tags = selected_tags(decoded.tags.as_deref());
    let rows = if selects_every_tag(tags.as_deref()) {
        store.daily_tag_spend(start_date, end_date, None).await
    } else {
        store
            .daily_tag_spend(start_date, end_date, tags.as_deref())
            .await
    };
    match rows {
        Ok(rows) => tag_spend_response(&rows),
        Err(error) => tag_failure(SPEND_TAGS_FAILED, &error),
    }
}

fn tag_spend_response(rows: &[DailyTagSpend]) -> Response {
    match tag_spend_body(rows) {
        Ok(body) => Json(body).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "the daily tag spend could not be aggregated");
            responses::wrapped_error(format!("{SPEND_TAGS_FAILED}{error}"))
        }
    }
}

fn tag_failure(prefix: &str, error: &TagReportError) -> Response {
    tracing::warn!(error = %error, "a tag report query failed");
    responses::wrapped_error(format!("{prefix}{error}"))
}
