use betterlite_http::url_policy::error::UrlPolicyError;
use betterlite_http::url_policy::origin::assert_same_origin;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    candidate_url: String,
    expected_url: String,
}

pub(crate) struct Op;

const fn class(error: &UrlPolicyError) -> &'static str {
    match error {
        UrlPolicyError::Parse(_) => "ValueError",
        UrlPolicyError::SchemeNotAllowed
        | UrlPolicyError::OriginScheme
        | UrlPolicyError::OriginHost
        | UrlPolicyError::OriginPort
        | UrlPolicyError::TooLong { .. }
        | UrlPolicyError::UnsupportedScheme { .. }
        | UrlPolicyError::InvalidUserinfo { .. } => "SSRFError",
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match assert_same_origin(&input.candidate_url, &input.expected_url) {
            // the oracle decodes a Python null as no value at all, so a passed check reports true on both sides
            Ok(_origin) => Ok(serde_json::json!(true)),
            Err(error) => Err(Box::new(HelperError {
                class: class(&error).to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })),
        }
    }
}
