use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::classifier_message_text::asks::current_ask_and_system_prompt;
use betterlite_router::classifier_message_text::markers::{
    MarkerPair, is_codex_user_agent, reminder_markers_for_request,
};
use betterlite_router::classifier_message_text::plan::matched_plan_mode_sentinel;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerPairInput {
    pub open: String,
    pub close: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub reminder_markers: Option<Vec<MarkerPairInput>>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub body: Option<Json>,
    #[serde(default)]
    pub plan_mode_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    pub ask: Option<String>,
    pub system_prompt: Option<String>,
    pub markers: Vec<MarkerPairInput>,
    pub plan_sentinel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let configured: Option<Vec<MarkerPair<'_>>> = input.reminder_markers.as_ref().map(|pairs| {
        pairs
            .iter()
            .map(|pair| MarkerPair {
                open: pair.open.as_str(),
                close: pair.close.as_str(),
            })
            .collect()
    });
    let codex = input.user_agent.as_deref().is_some_and(is_codex_user_agent);
    let markers = reminder_markers_for_request(configured.as_deref(), codex);
    let (ask, system_prompt) =
        current_ask_and_system_prompt(&input.messages, markers).map_err(invalid_request)?;
    let plan_sentinel = matched_plan_mode_sentinel(
        input.body.as_ref(),
        Some(&input.messages),
        &input.plan_mode_patterns,
        markers,
    )
    .map_err(invalid_request)?;
    let fields = Fields {
        ask: ask.map(|ask| ask.text),
        system_prompt: system_prompt.map(|prompt| prompt.text),
        markers: markers
            .iter()
            .map(|pair| MarkerPairInput {
                open: pair.open.to_owned(),
                close: pair.close.to_owned(),
            })
            .collect(),
        plan_sentinel,
    };
    Ok(ProbeOutput::ClassifierMessageText(Output(Box::new(fields))))
}

fn invalid_request(
    error: betterlite_router::classifier_message_text::error::ClassifierTextError,
) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
