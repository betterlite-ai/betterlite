use std::ops::Div as _;
use std::ops::Mul as _;

use num_traits::cast::cast;

use crate::dynamic_limits::ports::{CacheReadError, RouterReadError};

// check_available_usage's five-tuple: each limit's share for this project, what the model has left, and how many projects asked
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AvailableUsage {
    pub available_tpm: Option<i64>,
    pub available_rpm: Option<i64>,
    pub model_tpm: Option<i64>,
    pub model_rpm: Option<i64>,
    pub active_projects: Option<i64>,
}

// why a share cannot be computed: the two exceptions int() raises here, and the router read that failed earlier
#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("the handler holds no router, so no model group can be read")]
    MissingRouter,
    #[error("the active project count is zero, so there is no share to divide")]
    NoActiveProjects,
    #[error("the available quota is not a finite number")]
    NotFinite,
    #[error(transparent)]
    Cache(#[from] CacheReadError),
    #[error(transparent)]
    Router(#[from] RouterReadError),
}

// remaining_model_tpm: the model's total minus what it used this minute, or the total alone when its usage is unknown
#[must_use]
pub const fn remaining_limit(total: Option<i64>, current: Option<i64>) -> Option<i64> {
    let Some(limit) = total else {
        return None;
    };
    Some(match current {
        Some(used) => limit.saturating_sub(used),
        None => limit,
    })
}

// available quota: total * weight / active_projects, truncated toward zero like int() and then clamped to zero or positive
pub fn available_quota(
    remaining: Option<i64>,
    weight: f64,
    active_projects: Option<i64>,
) -> Result<Option<i64>, UsageError> {
    let Some(limit) = remaining else {
        return Ok(None);
    };
    let remaining_float = cast::<i64, f64>(limit).unwrap_or(f64::INFINITY);
    let scaled = remaining_float.mul(weight);
    let share = match active_projects {
        None => scaled,
        Some(0) => return Err(UsageError::NoActiveProjects),
        Some(count) => scaled.div(cast::<i64, f64>(count).unwrap_or(f64::INFINITY)),
    };
    if !share.is_finite() {
        return Err(UsageError::NotFinite);
    }
    let whole = cast::<f64, i64>(share).unwrap_or_else(|| saturated_quota(share));
    Ok(Some(whole.max(0)))
}

// int() has no bound in Python and i64 does: a finite share past the bound stops at it, and the clamp zeroes a negative one
const fn saturated_quota(share: f64) -> i64 {
    if share.is_sign_negative() {
        i64::MIN
    } else {
        i64::MAX
    }
}
