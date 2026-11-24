use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use betterlite_db::lens_repository::database::Database;
use betterlite_lens::models::execution::Sample;
use betterlite_lens::models::settings::LensSettings;
use betterlite_lens::sources::source_reader_sample::SampleWindow;
use betterlite_lens::sources::storage::Storage;
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use garde::Validate;
use serde::Deserialize;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::scope::{caller_key, user_scope};
use crate::lens_access::selection::validate_selection;
use crate::lens_access::state::LensRoutesState;
use crate::lens_access::storage::source_reader;
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::{LensMutationError, LensRouteError};

const PREVIEW_PAGE_SIZE: usize = 100;
const PREVIEW_LAG_MINUTES: i64 = 2;
const DEFAULT_LOOKBACK_HOURS: u32 = 24;

#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Preview {
    #[serde(default)]
    pub(crate) as_of: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    #[garde(range(min = 0))]
    pub(crate) offset: i64,
    #[garde(dive)]
    pub(crate) settings: LensSettings,
    #[serde(default = "default_lookback_hours")]
    #[garde(range(min = 1, max = 8760))]
    pub(crate) lookback_hours: u32,
}

const fn default_lookback_hours() -> u32 {
    DEFAULT_LOOKBACK_HOURS
}

pub(crate) async fn preview_sample<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Sample>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let preview: Preview = decode(&body)?;
    validate_selection(&preview.settings)?;
    let reader = source_reader(state.storage.as_ref())?;
    let scope = user_scope(&caller_key(&context.principal)?, false)?;
    let clock_now = state.clock.now();
    let now = preview
        .as_of
        .map_or(clock_now, |as_of| as_of.with_timezone(&Utc).min(clock_now));
    let start = TimeDelta::try_hours(i64::from(preview.lookback_hours))
        .and_then(|lookback| now.checked_sub_signed(lookback))
        .ok_or(LensMutationError::Deadline)?;
    let end = now
        .checked_sub_signed(TimeDelta::minutes(PREVIEW_LAG_MINUTES))
        .ok_or(LensMutationError::Deadline)?;
    let window = SampleWindow {
        start: start.timestamp_micros().div_euclid(1_000),
        end: end.timestamp_micros().div_euclid(1_000),
        offset: preview.offset,
        page_size: PREVIEW_PAGE_SIZE,
        preview: true,
        cursor: String::new(),
    };
    Ok(Json(
        reader.sample(&scope, &preview.settings, window).await?,
    ))
}

mod tests;
