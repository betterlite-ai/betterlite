use std::io::Error as IoError;

use thiserror::Error;

use super::wire::WireError;

#[derive(Debug, Error)]
pub enum PollingError {
    #[error("the proxy rejected the login session with HTTP {status}: {detail}")]
    Rejected { status: u16, detail: String },
    #[error(
        "the proxy rejected the login session with HTTP {status} and no error detail (from {url})"
    )]
    RejectedWithoutDetail { status: u16, url: String },
    #[error("the poll response does not match the poll data shape: {source}")]
    PollData {
        #[source]
        source: WireError,
    },
    #[error("the poll interval must be greater than zero")]
    ZeroPollInterval,
    #[error("the team details in the poll response are not valid: {source}")]
    TeamDetails {
        #[source]
        source: WireError,
    },
    #[error(
        "could not reach the proxy at {url}: {source}. Check that the proxy is running and that --base-url points at it."
    )]
    StartUnreachable {
        url: String,
        #[source]
        source: IoError,
    },
    #[error(
        "POST {url} returned HTTP {status}. Either --base-url is wrong, or the proxy is older than this CLI and does not support the CLI SSO login flow; upgrade the proxy or use a CLI version that matches it."
    )]
    StartUnsupported { status: u16, url: String },
    #[error("starting CLI login failed: HTTP {status} from {url}: {detail}")]
    StartFailedWithDetail {
        status: u16,
        url: String,
        detail: String,
    },
    #[error("starting CLI login failed: HTTP {status} from {url}")]
    StartFailed { status: u16, url: String },
    #[error("the proxy returned a non-JSON response from {url} (content-type: {content_type})")]
    StartNotJson { url: String, content_type: String },
    #[error(
        "the response from {url} is missing required field(s): {fields}. The proxy version may not match this CLI; upgrade whichever is older."
    )]
    StartMissingFields { url: String, fields: String },
    #[error("the response from {url} is not a valid CLI SSO start body: {source}")]
    StartBody {
        url: String,
        #[source]
        source: WireError,
    },
}
