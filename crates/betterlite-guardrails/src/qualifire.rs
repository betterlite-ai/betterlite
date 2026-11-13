pub mod convert;
pub mod error;
pub mod init;
mod request;
pub mod settings;
mod tests;
pub mod wire;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::Client;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_selection::gate::GuardrailSelection;
use error::QualifireError;
use init::QualifireArgs;
use request::CheckRequest;
use settings::{Check, OnFlagged, QualifireSettings, SecretLookup, insert_flag};

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

const MAX_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Debug, Clone)]
pub struct QualifireGuardrail {
    pub base: GuardrailBase,
    pub settings: QualifireSettings,
    pub client: Client,
}

impl QualifireGuardrail {
    pub fn new(
        args: &QualifireArgs,
        secrets: &dyn SecretLookup,
        environment: &dyn EnvironmentLookup,
        client: Client,
    ) -> Result<Self, QualifireError> {
        Ok(Self {
            base: GuardrailBase::new(guardrail_settings(args), true)?,
            settings: settings::resolve(args, secrets, environment)?,
            client,
        })
    }

    pub async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, QualifireError> {
        let dynamic = self.base.dynamic_request_body_params(request_data)?;
        let overrides = request::overrides(&dynamic)?;
        let Some((rows, output)) = request::select_rows(&inputs, request_data, input_type)? else {
            tracing::debug!("Qualifire Guardrail: No messages or texts found, skipping");
            return Ok(inputs);
        };
        let evaluation_id = overrides
            .evaluation_id
            .as_deref()
            .or(self.settings.evaluation_id.as_deref());
        let assertions = overrides
            .assertions
            .as_deref()
            .unwrap_or(self.settings.assertions.as_slice());
        let on_flagged = overrides.on_flagged.unwrap_or(self.settings.on_flagged);
        let check = CheckRequest {
            rows: &rows,
            output: output.as_deref(),
            evaluation_id,
            assertions,
            available_tools: request_data.get("tools"),
        };
        let plan = self.plan(&check)?;
        let verdict = self.send(&plan.url, &plan.payload()).await?;
        let response = verdict.response();
        let score = &response.score;
        let status = &response.status;
        tracing::debug!(
            "Qualifire Guardrail: Got result from API, score={score:?}, status={status:?}"
        );
        if verdict.is_flagged() {
            match on_flagged {
                OnFlagged::Monitor => tracing::warn!(
                    "Qualifire Guardrail: Monitoring mode - violation detected but allowing request. Response: {response:?}"
                ),
                OnFlagged::Block => {
                    return Err(QualifireError::Blocked {
                        detail: wire::BlockDetail {
                            error: "Violated guardrail policy".to_owned(),
                            qualifire_response: response,
                        },
                    });
                }
            }
        }
        Ok(inputs)
    }

    pub fn update_params(&mut self, params: &LitellmParams) -> Result<(), QualifireError> {
        let extra = &params.base.extra.0;
        let on_flagged = match extra.get("on_flagged") {
            Some(GuardrailValue::Text(text)) if !text.is_empty() => OnFlagged::parse(text)
                .ok_or_else(|| QualifireError::UnsupportedOnFlagged(text.clone()))?,
            Some(GuardrailValue::Text(_)) | None | Some(GuardrailValue::Null) => OnFlagged::Block,
            Some(wrong) => {
                let text = serde_json::to_string(wrong)
                    .map_err(|source| QualifireError::Encode { source })?;
                return Err(QualifireError::UnsupportedOnFlagged(text));
            }
        };
        self.settings.on_flagged = on_flagged;
        self.settings.api_key = params.base.fields.api_key.clone();
        if let Some(api_base) = &params.base.fields.api_base {
            self.settings.api_base.clone_from(api_base);
        }
        self.settings.evaluation_id = init::read(extra, "evaluation_id", None)?;
        self.settings.assertions = init::read(extra, "assertions", None)?.unwrap_or_default();
        self.settings.checks = checks_from_extra(extra)?;
        self.settings.timeout = match params.base.fields.timeout {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|source| QualifireError::InvalidTimeout { source })?,
            None => settings::DEFAULT_TIMEOUT,
        };
        self.base.update_params(params);
        Ok(())
    }
}

fn checks_from_extra(
    extra: &BTreeMap<String, GuardrailValue>,
) -> Result<BTreeSet<Check>, QualifireError> {
    let mut checks = BTreeSet::new();
    insert_flag(
        &mut checks,
        Check::PromptInjections,
        init::read(extra, "prompt_injections", None)?,
    );
    insert_flag(
        &mut checks,
        Check::Hallucinations,
        init::read(extra, "hallucinations_check", None)?,
    );
    insert_flag(
        &mut checks,
        Check::Grounding,
        init::read(extra, "grounding_check", None)?,
    );
    insert_flag(
        &mut checks,
        Check::Pii,
        init::read(extra, "pii_check", None)?,
    );
    insert_flag(
        &mut checks,
        Check::ContentModeration,
        init::read(extra, "content_moderation_check", None)?,
    );
    insert_flag(
        &mut checks,
        Check::ToolSelectionQuality,
        init::read(extra, "tool_selection_quality_check", None)?,
    );
    Ok(checks)
}

pub(super) fn guardrail_settings(args: &QualifireArgs) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: (!args.name.is_empty()).then(|| args.name.clone()),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(args.event_hook.clone()),
        default_on: args.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: args.timeout,
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: false,
            uses_apply_guardrail_interface: true,
            deny_text_on_error: false,
        },
    }
}
