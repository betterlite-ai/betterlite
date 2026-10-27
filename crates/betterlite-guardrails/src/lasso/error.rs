use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use super::wire::LassoResponse;

#[derive(Debug, Error)]
pub enum LassoError {
    #[error("the {name} header value for Lasso is not valid")]
    InvalidHeader {
        name: &'static str,
        source: InvalidHeaderValue,
    },
    #[error("the Lasso API did not answer in time")]
    Timeout { source: reqwest::Error },
    #[error("the Lasso API key was rejected")]
    InvalidApiKey,
    #[error("the Lasso API rate limit is exceeded")]
    RateLimited,
    #[error("the Lasso API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("could not check the request with the Lasso API")]
    Request { source: reqwest::Error },
    #[error("the Lasso API answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Lasso API answer is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Lasso blocked the content for {}", deputies.join(", "))]
    Blocked {
        deputies: Vec<String>,
        response: LassoResponse,
    },
}
