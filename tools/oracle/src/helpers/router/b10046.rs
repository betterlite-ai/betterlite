mod tests;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_router::io_token_reservations::limits::IoLimits;
use betterlite_router::remaining_usage_headers::{
    RemainingUsage,
    apply_remaining_usage_headers,
    // ast-grep-ignore: no-long-names
    build_io_token_rate_limit_headers,
};
use serde::{Deserialize, Serialize};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Build {
        itpm: Option<i64>,
        otpm: Option<i64>,
        current_itpm: Option<i64>,
        current_otpm: Option<i64>,
        now: u64,
    },
    Apply {
        headers: AdditionalHeaders,
        remaining: RemainingUsage,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Build {
                itpm,
                otpm,
                current_itpm,
                current_otpm,
                now,
            } => {
                // ast-grep-ignore: no-long-names
                let headers = build_io_token_rate_limit_headers(
                    IoLimits {
                        itpm: *itpm,
                        otpm: *otpm,
                    },
                    *current_itpm,
                    *current_otpm,
                    instant(*now)?,
                );
                to_json(headers)
            }
            Input::Apply { headers, remaining } => {
                let mut merged = headers.clone();
                apply_remaining_usage_headers(&mut merged, remaining);
                to_json(merged)
            }
        }
    }
}

fn instant(now: u64) -> Result<SystemTime, Box<HelperError>> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(now))
        .ok_or_else(|| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: format!("the instant {now} is past what SystemTime holds"),
            })
        })
}

fn to_json<T: Serialize>(value: T) -> HelperResult {
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
