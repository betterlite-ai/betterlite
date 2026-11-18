use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

pub(crate) const MAX_GUARDRAILS: usize = 32;
pub(crate) const MAX_RUNS: usize = 32;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

// LiteLLM's apply_guardrail result for realtime text: a clean or rewritten text list, or one of the two exception shapes
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    #[default]
    Clean,
    Rewrite,
    Raise,
}

// The exception a raising guardrail throws: LiteLLM blocks on a status_code attribute or a ValueError, and re-raises anything else
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Raised {
    pub(crate) message: String,
    pub(crate) detail: Option<Json>,
    pub(crate) status_code: Option<u16>,
    pub(crate) value_error: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct PolicyDocument {
    pub(crate) violation_message: Option<String>,
    pub(crate) on_violation: Option<OnViolation>,
    pub(crate) end_session_after_n_fails: Option<i64>,
}

// The callback object the case installs: a CustomGuardrail, one with native lifecycle hooks, or a plain logger
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GuardrailKind {
    Custom,
    Native,
    #[default]
    Plain,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct GuardrailDocument {
    pub(crate) name: Option<String>,
    pub(crate) kind: GuardrailKind,
    pub(crate) hooks: Vec<GuardrailEventHooks>,
    pub(crate) default_on: bool,
    pub(crate) outcome: Outcome,
    pub(crate) raised: Option<Raised>,
    pub(crate) policy: PolicyDocument,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RunDocument {
    pub(crate) transcript: String,
    pub(crate) item_id: Option<String>,
    pub(crate) pre_block_backend_message: Option<String>,
    pub(crate) event_hooks: Option<Vec<GuardrailEventHooks>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Input {
    pub(crate) request_data: IndexMap<String, Json>,
    pub(crate) user_api_key_dict: Option<Json>,
    pub(crate) guardrails: Vec<GuardrailDocument>,
    pub(crate) callbacks: Vec<usize>,
    pub(crate) backend_failures: Vec<usize>,
    pub(crate) client_failures: Vec<usize>,
    pub(crate) close_failure: bool,
    pub(crate) runs: Vec<RunDocument>,
}

impl Input {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.guardrails.len() > MAX_GUARDRAILS
            || self.callbacks.len() > MAX_GUARDRAILS
            || self.runs.len() > MAX_RUNS
            || self.backend_failures.len() > MAX_RUNS
            || self.client_failures.len() > MAX_RUNS
        {
            return Err(String::from(
                "the scripted guardrails input is past its limits",
            ));
        }
        if self
            .callbacks
            .iter()
            .any(|index| *index >= self.guardrails.len())
        {
            return Err(String::from("a callback index is past the guardrails"));
        }
        let sizes = self.sizes()?;
        if sizes.iter().any(|size| *size > MAX_TEXT_BYTES)
            || sizes
                .iter()
                .fold(0_usize, |total, size| total.saturating_add(*size))
                > MAX_CAPTURE_BYTES
        {
            return Err(String::from(
                "the scripted guardrails input is past its byte limit",
            ));
        }
        Ok(())
    }

    fn sizes(&self) -> Result<Vec<usize>, String> {
        let texts = self.runs.iter().flat_map(|run| {
            [
                run.transcript.len(),
                run.item_id.as_ref().map_or(0, String::len),
                run.pre_block_backend_message
                    .as_ref()
                    .map_or(0, String::len),
            ]
        });
        let mut sizes: Vec<usize> = texts.collect();
        for guardrail in &self.guardrails {
            sizes.push(guardrail.name.as_ref().map_or(0, String::len));
            sizes.push(
                guardrail
                    .policy
                    .violation_message
                    .as_ref()
                    .map_or(0, String::len),
            );
            if let Some(raised) = &guardrail.raised {
                sizes.push(raised.message.len());
            }
        }
        let data = serde_json::to_string(&self.request_data).map_err(|error| error.to_string())?;
        sizes.push(data.len());
        if let Some(principal) = &self.user_api_key_dict {
            let principal = serde_json::to_string(principal).map_err(|error| error.to_string())?;
            sizes.push(principal.len());
        }
        Ok(sizes)
    }
}
