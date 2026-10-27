use super::messages::{Messages, Output, parse_messages};
use super::params::Cleaned;
use super::wire::{Metadata, Tokens, Usage};
use chrono::{DateTime, SecondsFormat, Utc};
use pep440_rs::{Version, VersionParseError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LunaryError {
    #[error("invalid Lunary version")]
    InvalidVersion {
        #[source]
        source: VersionParseError,
    },
    #[error("Lunary version {version} is outdated; required >= 0.1.43")]
    OutdatedVersion { version: String },
}

pub fn check_version(version: &str) -> Result<(), LunaryError> {
    let parsed = version
        .parse::<Version>()
        .map_err(|source| LunaryError::InvalidVersion { source })?;
    if parsed < Version::new([0, 1, 43]) {
        return Err(LunaryError::OutdatedVersion {
            version: version.to_owned(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Request<Content, Arguments, Fields, Tools> {
    pub event_type: String,
    pub run_id: String,
    pub model: String,
    pub input: Option<Messages<Content, Arguments>>,
    pub metadata: Metadata<Fields>,
    pub params: Cleaned<Tools>,
    pub template_id: Option<String>,
    pub user_id: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Response<Content, Arguments> {
    pub choices: Option<Messages<Content, Arguments>>,
    pub usage: Option<Usage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Event<Content, Arguments, Fields, Tools> {
    #[serde(rename = "type")]
    pub event_type: String,
    pub event: String,
    pub run_id: String,
    pub timestamp: String,
    pub runtime: String,
    #[serde(flatten)]
    pub data: Data<Content, Arguments, Fields, Tools>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Data<Content, Arguments, Fields, Tools> {
    Start {
        parent_run_id: Option<String>,
        user_id: Option<String>,
        name: String,
        input: Option<Output<Content, Arguments>>,
        template_id: Option<String>,
        metadata: Metadata<Fields>,
        tags: Option<Vec<String>>,
        params: Cleaned<Tools>,
    },
    Terminal {
        error: Option<Failure>,
        output: Option<Output<Content, Arguments>>,
        token_usage: Option<Tokens>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub stack: String,
}

pub fn events<Content, Arguments, Fields, Tools>(
    request: &Request<Content, Arguments, Fields, Tools>,
    response: Option<&Response<Content, Arguments>>,
    terminal: &str,
    error: Option<&str>,
) -> [Event<Content, Arguments, Fields, Tools>; 2]
where
    Content: Clone,
    Arguments: Clone,
    Fields: Clone,
    Tools: Clone,
{
    let start = Event {
        event_type: request.event_type.clone(),
        event: "start".to_owned(),
        run_id: request.run_id.clone(),
        timestamp: timestamp(request.start_time),
        runtime: "litellm".to_owned(),
        data: start_data(request),
    };
    let end = Event {
        event_type: request.event_type.clone(),
        event: terminal.to_owned(),
        run_id: request.run_id.clone(),
        timestamp: timestamp(request.end_time),
        runtime: "litellm".to_owned(),
        data: Data::Terminal {
            error: error.filter(|text| !text.is_empty()).map(|stack| Failure {
                stack: stack.to_owned(),
            }),
            output: response
                .and_then(|body| body.choices.as_ref())
                .map(parse_messages),
            token_usage: response
                .and_then(|body| body.usage.as_ref())
                .map(Tokens::from),
        },
    };
    [start, end]
}

fn start_data<Content, Arguments, Fields, Tools>(
    request: &Request<Content, Arguments, Fields, Tools>,
) -> Data<Content, Arguments, Fields, Tools>
where
    Content: Clone,
    Arguments: Clone,
    Fields: Clone,
    Tools: Clone,
{
    let metadata = request.metadata.clone();
    Data::Start {
        parent_run_id: metadata.parent_run_id.clone(),
        user_id: request.user_id.clone(),
        name: request.model.clone(),
        input: request.input.as_ref().map(parse_messages),
        template_id: request.template_id.clone(),
        tags: metadata.tags.clone(),
        metadata,
        params: request.params.clone(),
    }
}

fn timestamp(time: DateTime<Utc>) -> String {
    // Python datetime.isoformat uses six fractional digits when microseconds are nonzero.
    let precision = if time.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    time.to_rfc3339_opts(precision, false)
}
