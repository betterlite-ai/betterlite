use std::convert::Infallible;
use std::fmt::Display;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::realtime_guardrail_policy::{
    RealtimeGuardrailCallback, TRANSCRIPTION_EVENT_HOOKS, disable_auto_response_message,
    guardrails_run_for_hooks, inject_auto_response_disable,
};
use betterlite_core::api::request_destination::RequestData;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallbackSpec {
    custom: bool,
    runs_for: Vec<GuardrailEventHooks>,
}

impl RealtimeGuardrailCallback for CallbackSpec {
    type Error = Infallible;

    fn is_custom_guardrail(&self) -> bool {
        self.custom
    }

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Infallible> {
        _ = data;
        Ok(self.runs_for.contains(&event))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    callbacks: Vec<CallbackSpec>,
    request_data: RequestData,
    event_hooks: Vec<GuardrailEventHooks>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    sent: bool,
    callbacks: Vec<CallbackSpec>,
    request_data: RequestData,
    setup_messages: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "transcription_guardrail_selection")]
    TranscriptionSelection {
        scenarios: Vec<Scenario>,
    },
    DisableAutoResponseShapes {
        backend_uses_beta_protocol: Vec<bool>,
    },
    SetupGuardrailInjection {
        sessions: Vec<Session>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::TranscriptionSelection { scenarios } => {
                let mut answers = Vec::with_capacity(scenarios.len());
                for scenario in scenarios {
                    let callbacks: Vec<&CallbackSpec> = scenario.callbacks.iter().collect();
                    let Ok(audio) = guardrails_run_for_hooks(
                        &callbacks,
                        &scenario.request_data,
                        &TRANSCRIPTION_EVENT_HOOKS,
                    );
                    let Ok(hooks) = guardrails_run_for_hooks(
                        &callbacks,
                        &scenario.request_data,
                        &scenario.event_hooks,
                    );
                    answers.push(json!({ "audio": audio, "hooks": hooks }));
                }
                Ok(json!({ "scenarios": answers }))
            }
            Input::DisableAutoResponseShapes {
                backend_uses_beta_protocol,
            } => {
                let mut messages = Vec::with_capacity(backend_uses_beta_protocol.len());
                for beta in backend_uses_beta_protocol {
                    let text = disable_auto_response_message(*beta).map_err(failure)?;
                    messages.push(serde_json::from_str::<Value>(&text).map_err(failure)?);
                }
                Ok(json!({ "messages": messages }))
            }
            Input::SetupGuardrailInjection { sessions } => {
                let mut answers = Vec::with_capacity(sessions.len());
                for session in sessions {
                    let callbacks: Vec<&CallbackSpec> = session.callbacks.iter().collect();
                    let mut sent = session.sent;
                    let mut messages = Vec::with_capacity(session.setup_messages.len());
                    for text in &session.setup_messages {
                        let injected = inject_auto_response_disable(
                            text,
                            &mut sent,
                            &callbacks,
                            &session.request_data,
                        )
                        .map_err(failure)?;
                        let output = injected.as_deref().unwrap_or(text);
                        messages.push(json!({
                            "changed": injected.is_some(),
                            "sent": sent,
                            "message": serde_json::from_str::<Value>(output).ok(),
                        }));
                    }
                    answers.push(json!({ "sent": sent, "messages": messages }));
                }
                Ok(json!({ "sessions": answers }))
            }
        }
    }
}

fn failure<Failed>(error: Failed) -> Box<HelperError>
where
    Failed: Display,
{
    Box::new(HelperError {
        class: "RealtimeGuardrailPolicyError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
