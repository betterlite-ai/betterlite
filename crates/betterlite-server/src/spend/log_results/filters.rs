use betterlite_auth::user_roles::UserRole;
use betterlite_db::key_user_writes::hash::hash_token_if_needed;
use chrono::NaiveDate;
use secrecy::ExposeSecret as _;

use super::dates::{LegacyDateError, parse_legacy_date};
use super::query::LegacySpendQuery;

// both dates together: the window runs from UTC midnight of the first day through UTC midnight of the last
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyWindow {
    pub start_iso: String,
    pub end_iso: String,
    pub end_date: NaiveDate,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct LegacyFilters {
    // already hashed when the caller sent an sk- key
    pub api_key: Option<String>,
    pub request_id: Option<String>,
    pub user_id: Option<String>,
    pub window: Option<LegacyWindow>,
}

impl LegacyFilters {
    // the filters view_spend_logs builds, with user_id already narrowed for an internal caller
    pub fn resolve(
        query: &LegacySpendQuery,
        user_id: Option<String>,
    ) -> Result<Self, LegacyDateError> {
        Ok(Self {
            // only an sk- key is hashed before it reaches the api_key column
            api_key: query
                .api_key
                .as_ref()
                .map(|secret| hash_token_if_needed(secret.expose_secret())),
            request_id: query.request_id.clone(),
            user_id,
            window: legacy_window(query.start_date.as_deref(), query.end_date.as_deref())?,
        })
    }
}

// an internal user reads only their own rows; every other role keeps the user_id the caller asked for
pub fn scoped_user_id(
    role: UserRole,
    caller_user_id: Option<&str>,
    requested_user_id: Option<&str>,
) -> Option<String> {
    if role.is_internal_user_role() {
        caller_user_id.map(str::to_owned)
    } else {
        requested_user_id.map(str::to_owned)
    }
}

fn legacy_window(
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<Option<LegacyWindow>, LegacyDateError> {
    let (Some(start_text), Some(end_text)) = (start_date, end_date) else {
        return Ok(None);
    };
    let start = parse_legacy_date(start_text)?;
    let end = parse_legacy_date(end_text)?;
    Ok(Some(LegacyWindow {
        start_iso: format!("{}T00:00:00+00:00", start.format("%Y-%m-%d")),
        end_iso: format!("{}T00:00:00+00:00", end.format("%Y-%m-%d")),
        end_date: end,
    }))
}
