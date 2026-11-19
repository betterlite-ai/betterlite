mod scenario;

use betterlite_router::affinity_claims::identity::AffinityKey;
use betterlite_router::affinity_claims::pin::{AffinityPin, PinError, StoredPin};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// twice the parser's 1 MiB bound, so one case reaches the rejection without a 1 MiB case file
pub(super) const MAX_PARSE_PAD: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub shared: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Key {
        request: KeyRequest,
    },
    Claim {
        key: String,
        pin: Vec<PinField>,
        ttl_seconds: u32,
        #[serde(default)]
        down: bool,
        #[serde(default)]
        eligible: Option<Vec<Vec<PinField>>>,
    },
    Seed {
        target: SeedTarget,
        key: String,
        raw: String,
        ttl_seconds: u32,
    },
    Parse {
        pad_len: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRequest {
    pub kind: KeyKind,
    pub model_group: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub user_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Affinity,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedTarget {
    Local,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<Observation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observation {
    Key {
        key: Option<String>,
    },
    Claim {
        pin_kind: String,
        model_id: Option<String>,
        text: Option<String>,
        fallback: Option<String>,
    },
    Seed,
    Parse {
        shape: String,
        byte_count: usize,
        reason: Option<String>,
    },
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime.block_on(scenario::execute(input))?;
    Ok(ProbeOutput::AffinityClaims(output))
}

// summarize what StoredPin::parse made of a generated pin, so a rejected 1 MiB input stays a small reply
pub(super) fn parse_step(pad_len: u64) -> Result<Observation, OperationError> {
    if pad_len > MAX_PARSE_PAD {
        return Err(invalid(
            "the parse pad is past the 2097152-byte build limit",
        ));
    }
    let repetitions = usize::try_from(pad_len).map_err(|error| invalid(&error.to_string()))?;
    let text = format!("{{\"model_id\":\"{}\"}}", "a".repeat(repetitions));
    let (shape, byte_count, reason) = match StoredPin::parse(text.as_bytes()) {
        Ok(StoredPin::Pin(pin)) => (
            "pin".to_owned(),
            pin.0.get("model_id").map_or(0, String::len),
            None,
        ),
        Ok(StoredPin::Text(value)) => ("text".to_owned(), value.len(), None),
        Ok(StoredPin::Other(_value)) => ("other".to_owned(), 0, None),
        Err(PinError::TooLarge { .. }) => ("rejected".to_owned(), 0, Some("too_large".to_owned())),
        Err(PinError::NotText | PinError::NotFields) => {
            return Err(invalid("the parser rejected the generated pin"));
        }
    };
    Ok(Observation::Parse {
        shape,
        byte_count,
        reason,
    })
}

pub(super) fn claim_fields(stored: &StoredPin) -> (String, Option<String>, Option<String>) {
    match stored {
        StoredPin::Pin(pin) => ("pin".to_owned(), pin.0.get("model_id").cloned(), None),
        StoredPin::Text(text) => ("text".to_owned(), None, Some(text.clone())),
        StoredPin::Other(_value) => ("other".to_owned(), None, None),
    }
}

pub(super) fn build_pin(fields: &[PinField]) -> AffinityPin {
    let mut map = IndexMap::with_capacity(fields.len());
    for field in fields {
        map.insert(field.name.clone(), field.value.clone());
    }
    AffinityPin(map)
}

pub(super) fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
