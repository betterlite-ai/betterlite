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
use super::query::decode_tag_report_query;
use super::responses::{self, NO_DATABASE};
use super::rows::{DailyTagSpend, KeyAliasedUser, TagNamesBody};
use super::store::{TagReportError, TagReportHandle, TagReportStore};

const SPEND_TAGS_FAILED: &str = "/spend/tags Error";
const TAG_NAMES_FAILED: &str = "/spend/all_tag_names Error";
const FORBIDDEN_USER: &str = "Not authorized to view spend for another user.";

#[derive(Clone, Copy)]
pub(super) enum TableReport {
    Keys,
    Users,
}

#[derive(Clone, Copy)]
pub(super) enum TagReport {
    Tags,
    TagNames,
    DailyTags,
}

impl TagReport {
    // the legacy tag route and the UI tag route both answer "/spend/tags Error"
    const fn prefix(self) -> &'static str {
        match self {
            Self::Tags | Self::DailyTags => SPEND_TAGS_FAILED,
            Self::TagNames => TAG_NAMES_FAILED,
        }
    }

    async fn answer(self, store: &dyn TagReportStore, raw_query: Option<&str>) -> Response {
        match self {
            Self::Tags => match store.raw_tag_spend().await {
                Ok(rows) => Json(rows).into_response(),
                Err(error) => tag_failure(self.prefix(), &error),
            },
            Self::TagNames => match store.tag_names().await {
                Ok(Some(tag_names)) => Json(TagNamesBody { tag_names }).into_response(),
                Ok(None) => Json(Vec::<Option<String>>::new()).into_response(),
                Err(error) => tag_failure(self.prefix(), &error),
            },
            Self::DailyTags => daily_tags(store, raw_query.unwrap_or_default()).await,
        }
    }
}

// spend_key_fn and spend_user_fn: the admin predicate picks the whole table or the caller's own
pub(super) async fn table_respond(
    State(store): State<TagReportHandle>,
    Extension(report): Extension<TableReport>,
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
    let query = decode_tag_report_query(raw_query.as_deref().unwrap_or_default().as_bytes());
    match report {
        TableReport::Keys => {
            if is_admin_view_safe(&caller) {
                return key_rows(store.keys().await);
            }
            match caller.user_id.as_deref().filter(|id| !id.is_empty()) {
                Some(user_id) => key_rows(store.keys_for_user(user_id).await),
                None => Json(Vec::<LiteLLMVerificationToken>::new()).into_response(),
            }
        }
        TableReport::Users => {
            if !is_admin_view_safe(&caller) {
                return scoped_user_rows(store, &caller, query.user_id.as_deref()).await;
            }
            match query.user_id.as_deref() {
                Some(user_id) => single_user(store.user(user_id).await),
                None => all_user_rows(store.users().await),
            }
        }
    }
}

// view_spend_tags, global_get_all_tag_names and global_view_spend_tags are not tenant scoped and fail through their own ProxyException wrapper
pub(super) async fn tag_respond(
    State(store): State<TagReportHandle>,
    Extension(report): Extension<TagReport>,
    identity: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    if identity.is_none() {
        return unauthenticated();
    }
    let Some(store) = store.as_deref() else {
        return responses::wrapped_error(format!("{}{NO_DATABASE}", report.prefix()));
    };
    report.answer(store, raw_query.as_deref()).await
}

fn key_rows(result: Result<Vec<LiteLLMVerificationToken>, TagReportError>) -> Response {
    match result {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => responses::detail_error(StatusCode::BAD_REQUEST, &error.to_string()),
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

// ui_get_spend_by_tags: both dates decide the query, and only a raised ProxyException skips it
async fn daily_tags(store: &dyn TagReportStore, raw_query: &str) -> Response {
    let query = decode_tag_report_query(raw_query.as_bytes());
    let (Some(start_date), Some(end_date)) =
        (query.start_date.as_deref(), query.end_date.as_deref())
    else {
        return responses::missing_dates();
    };
    let tags = query.selected_tags();
    let rows = if query.selects_every_tag() {
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
