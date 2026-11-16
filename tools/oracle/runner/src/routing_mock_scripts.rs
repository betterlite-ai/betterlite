use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::time::Duration;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::case::UpstreamAnswer;
use crate::error::HelperRunError;
use crate::mock::MAX_BODY;
use crate::mock::ProviderRequest;

pub mod run;
mod tests;

pub const MAX_REPLIES: usize = 256;
pub const MAX_TRACE: usize = 1024;
// the oracle's HTTP client gives up after 30 seconds, so no wait may outlast it
const MAX_DEADLINE_MS: u64 = 30_000;
const CREDENTIAL_HEADERS: [&str; 9] = [
    "authorization",
    "proxy-authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "x-litellm-api-key",
    "x-amz-security-token",
    "cookie",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSpec {
    pub deadline_ms: u64,
    pub replies: Vec<ScriptedReply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedReply {
    #[serde(default)]
    pub wait: Wait,
    pub then: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Wait {
    #[default]
    None,
    Delay {
        millis: u64,
    },
    Barrier {
        name: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Answer(UpstreamAnswer),
    Close,
}

#[derive(Debug, Clone)]
pub struct MockScript {
    pub deadline: Duration,
    pub replies: Vec<ScriptedReply>,
    pub barriers: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderCall {
    pub request: ProviderRequest,
    pub outcome: CallOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Waiting,
    Answered { status: u16 },
    Closed,
    Cancelled,
    DeadlineExceeded,
    Exhausted,
}

pub fn parse_script(bytes: &[u8]) -> Result<MockScript, HelperRunError> {
    if bytes.len() > MAX_BODY {
        return Err(HelperRunError::ScriptTooLarge { bytes: bytes.len() });
    }
    let spec = serde_json::from_slice::<ScriptSpec>(bytes).map_err(HelperRunError::ScriptParse)?;
    MockScript::try_from(spec)
}

impl TryFrom<ScriptSpec> for MockScript {
    type Error = HelperRunError;

    fn try_from(spec: ScriptSpec) -> Result<Self, HelperRunError> {
        if spec.replies.len() > MAX_REPLIES {
            return Err(HelperRunError::ScriptReplies {
                count: spec.replies.len(),
            });
        }
        if !(1..=MAX_DEADLINE_MS).contains(&spec.deadline_ms) {
            return Err(HelperRunError::ScriptDeadline {
                millis: spec.deadline_ms,
            });
        }
        let mut barriers = BTreeMap::<String, usize>::new();
        for (index, reply) in spec.replies.iter().enumerate() {
            check_reply(index, reply, spec.deadline_ms)?;
            if let Wait::Barrier { name } = &reply.wait {
                let parties = barriers.entry(name.clone()).or_default();
                *parties = parties.saturating_add(1);
            }
        }
        if let Some(name) = barriers
            .iter()
            .find_map(|(name, parties)| (*parties < 2).then_some(name))
        {
            return Err(HelperRunError::ScriptBarrier { name: name.clone() });
        }
        Ok(Self {
            deadline: Duration::from_millis(spec.deadline_ms),
            replies: spec.replies,
            barriers,
        })
    }
}

impl TryFrom<&[UpstreamAnswer]> for MockScript {
    type Error = HelperRunError;

    fn try_from(answers: &[UpstreamAnswer]) -> Result<Self, HelperRunError> {
        let replies = answers
            .iter()
            .map(|answer| ScriptedReply {
                wait: Wait::None,
                then: Outcome::Answer(answer.clone()),
            })
            .collect();
        Self::try_from(ScriptSpec {
            deadline_ms: MAX_DEADLINE_MS,
            replies,
        })
    }
}

fn check_reply(
    index: usize,
    reply: &ScriptedReply,
    deadline_ms: u64,
) -> Result<(), HelperRunError> {
    if let Wait::Delay { millis } = &reply.wait
        && *millis > deadline_ms
    {
        return Err(HelperRunError::ScriptDelay {
            index,
            millis: *millis,
        });
    }
    if let Outcome::Answer(answer) = &reply.then {
        if answer.body.is_some() == answer.events.is_some() {
            return Err(HelperRunError::ScriptAnswer { index });
        }
        StatusCode::from_u16(answer.status)
            .map_err(|source| HelperRunError::ScriptStatus { index, source })?;
    }
    Ok(())
}

pub fn redact_credentials(headers: &mut BTreeMap<String, String>) {
    for name in CREDENTIAL_HEADERS {
        if let Some(value) = headers.get_mut(name) {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            *value = format!("redacted:{:016x}", hasher.finish());
        }
    }
}
