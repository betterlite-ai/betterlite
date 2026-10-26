mod tests;

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgenticKwargValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgenticFollowupKwargsError {
    #[error("agentic loop depth overflowed")]
    DepthOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgenticFollowupKwargs {
    values: BTreeMap<String, AgenticKwargValue>,
}

pub struct AgenticFollowupInput<'values> {
    pub request_kwargs: &'values BTreeMap<String, AgenticKwargValue>,
    pub patch_kwargs: &'values BTreeMap<String, AgenticKwargValue>,
    pub request_params: &'values BTreeSet<String>,
    pub depth: u32,
    pub max_loops: u32,
    pub fingerprints: &'values [String],
    pub fingerprint: &'values str,
}

impl AsRef<BTreeMap<String, AgenticKwargValue>> for AgenticFollowupKwargs {
    fn as_ref(&self) -> &BTreeMap<String, AgenticKwargValue> {
        &self.values
    }
}

pub fn build_agentic_followup_kwargs(
    input: &AgenticFollowupInput<'_>,
) -> Result<AgenticFollowupKwargs, AgenticFollowupKwargsError> {
    let next_depth = input
        .depth
        .checked_add(1)
        .ok_or(AgenticFollowupKwargsError::DepthOverflow)?;
    let mut values = input
        .request_kwargs
        .iter()
        .chain(input.patch_kwargs)
        .filter(|(key, _)| !input.request_params.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = input.fingerprints.to_vec();
    seen.push(input.fingerprint.to_owned());
    values.insert(
        "_agentic_loop_depth".to_owned(),
        AgenticKwargValue::Integer(i64::from(next_depth)),
    );
    values.insert(
        "max_agentic_loops".to_owned(),
        AgenticKwargValue::Integer(i64::from(input.max_loops)),
    );
    values.insert(
        "_agentic_loop_fingerprints".to_owned(),
        AgenticKwargValue::Array(seen.into_iter().map(AgenticKwargValue::Text).collect()),
    );
    Ok(AgenticFollowupKwargs { values })
}
