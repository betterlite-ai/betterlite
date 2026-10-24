use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::time::Duration;

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::OracleError;
use crate::case::UpstreamAnswer;
use crate::http::MAX_BODY;
use crate::mock::ProviderRequest;

mod run;
mod tests;

pub(crate) use run::{CallGuard, Recording, Ticket};

pub(crate) const MAX_REPLIES: usize = 256;
pub(crate) const MAX_TRACE: usize = 1024;
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
pub(crate) struct ScriptSpec {
    pub(crate) deadline_ms: u64,
    pub(crate) replies: Vec<ScriptedReply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedReply {
    #[serde(default)]
    pub(crate) wait: Wait,
    pub(crate) then: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Wait {
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
pub(crate) enum Outcome {
    Answer(UpstreamAnswer),
    Close,
}

#[derive(Debug, Clone)]
pub struct MockScript {
    pub(crate) deadline: Duration,
    pub(crate) replies: Vec<ScriptedReply>,
    pub(crate) barriers: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderCall {
    pub(crate) request: ProviderRequest,
    pub(crate) outcome: CallOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CallOutcome {
    Waiting,
    Answered { status: u16 },
    Closed,
    Cancelled,
    DeadlineExceeded,
    Exhausted,
}

pub fn parse_script(bytes: &[u8]) -> Result<MockScript, OracleError> {
    if bytes.len() > MAX_BODY {
        return Err(OracleError::ScriptTooLarge { bytes: bytes.len() });
    }
    let spec = serde_json::from_slice::<ScriptSpec>(bytes).map_err(OracleError::ScriptParse)?;
    MockScript::try_from(spec)
}

impl TryFrom<ScriptSpec> for MockScript {
    type Error = OracleError;

    fn try_from(spec: ScriptSpec) -> Result<Self, OracleError> {
        if spec.replies.len() > MAX_REPLIES {
            return Err(OracleError::ScriptReplies {
                count: spec.replies.len(),
            });
        }
        if !(1..=MAX_DEADLINE_MS).contains(&spec.deadline_ms) {
            return Err(OracleError::ScriptDeadline {
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
            return Err(OracleError::ScriptBarrier { name: name.clone() });
        }
        Ok(Self {
            deadline: Duration::from_millis(spec.deadline_ms),
            replies: spec.replies,
            barriers,
        })
    }
}

impl TryFrom<&[UpstreamAnswer]> for MockScript {
    type Error = OracleError;

    fn try_from(answers: &[UpstreamAnswer]) -> Result<Self, OracleError> {
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

fn check_reply(index: usize, reply: &ScriptedReply, deadline_ms: u64) -> Result<(), OracleError> {
    if let Wait::Delay { millis } = &reply.wait
        && *millis > deadline_ms
    {
        return Err(OracleError::ScriptDelay {
            index,
            millis: *millis,
        });
    }
    if let Outcome::Answer(answer) = &reply.then {
        if answer.body.is_some() == answer.events.is_some() {
            return Err(OracleError::ScriptAnswer { index });
        }
        StatusCode::from_u16(answer.status)
            .map_err(|source| OracleError::ScriptStatus { index, source })?;
    }
    Ok(())
}

pub(crate) fn redact(headers: &mut BTreeMap<String, String>) {
    for name in CREDENTIAL_HEADERS {
        if let Some(value) = headers.get_mut(name) {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            *value = format!("redacted:{:016x}", hasher.finish());
        }
    }
}
