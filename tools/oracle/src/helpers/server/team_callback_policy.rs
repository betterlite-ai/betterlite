use std::sync::PoisonError;

use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::callback_vars::{Transform, transform_callback_vars};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use betterlite_server::member_request_contracts::callbacks::TeamCallbackMetadata;
use betterlite_server::team_callback_policy::error::{
    ErrorDetail, TeamCallbackError, unknown_team_error,
};
use betterlite_server::team_callback_policy::redaction::redact_callback_secrets;
use betterlite_server::team_callback_policy::resolve::resolve_team_callbacks;
use betterlite_server::team_callback_policy::validate_team_callback;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use audit_case::{AuditAnswer, AuditCase, audit};
use warnings::Warnings;

mod audit_case;
mod warnings;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Validate {
        callback_name: String,
        callback_vars: CallbackVars,
    },
    Redact {
        metadata: Json,
    },
    Resolve {
        metadata: Json,
        encrypt: bool,
        salt_key: String,
    },
    Audit(AuditCase),
    UnknownTeam {
        team_id: String,
        user_role: Option<UserRole>,
        status: u16,
    },
}

#[derive(Debug, Serialize)]
struct StepResult {
    value: Answer,
    error: Option<ErrorOut>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Answer {
    Nothing,
    Redacted { redacted: Json, original: Json },
    Callbacks(TeamCallbackMetadata),
    Audit(AuditAnswer),
}

#[derive(Debug, Serialize)]
struct ErrorOut {
    status: Option<u16>,
    detail: Option<ErrorDetail>,
}

type StepOutcome = Result<Answer, TeamCallbackError>;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let mut results = Vec::new();
        for step in &input.steps {
            let warnings = Warnings::default();
            let subscriber = tracing_subscriber::registry().with(warnings.clone());
            let outcome = with_default(subscriber, || run_step(step, &runtime))?;
            let lines = warnings
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            results.push(match outcome {
                Ok(value) => StepResult {
                    value,
                    error: None,
                    warnings: lines,
                },
                Err(error) => {
                    let failure = error.failure();
                    let status = failure.as_ref().map(|http| http.status.as_u16());
                    let detail = failure.map(|http| http.detail);
                    StepResult {
                        value: Answer::Nothing,
                        error: Some(ErrorOut { status, detail }),
                        warnings: lines,
                    }
                }
            });
        }
        Ok(serde_json::to_value(results)?)
    }
}

fn run_step(step: &Step, runtime: &Runtime) -> Result<StepOutcome, Box<HelperError>> {
    Ok(match step {
        Step::Validate {
            callback_name,
            callback_vars,
        } => {
            let flags = FeatureFlags {
                otel_v2_enabled: false,
            };
            validate_team_callback(callback_name, callback_vars, flags).map(|()| Answer::Nothing)
        }
        Step::Redact { metadata } => Ok(Answer::Redacted {
            redacted: redact_callback_secrets(metadata),
            original: metadata.clone(),
        }),
        Step::Resolve {
            metadata,
            encrypt,
            salt_key,
        } => resolve(metadata, *encrypt, salt_key)?,
        Step::Audit(case) => audit(case, runtime).map(Answer::Audit),
        Step::UnknownTeam {
            team_id,
            user_role,
            status,
        } => {
            let code = StatusCode::from_u16(*status).or_value_error()?;
            Err(unknown_team_error(team_id, *user_role, code))
        }
    })
}

// the case's metadata is stored the way the write path stores it, encrypted when the case says so
fn resolve(
    metadata: &Json,
    encrypt: bool,
    salt_key: &str,
) -> Result<StepOutcome, Box<HelperError>> {
    let stored_cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt_key)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    let stored = if encrypt {
        let plain = serde_json::to_value(metadata)?;
        serde_json::from_value(transform_callback_vars(
            &plain,
            &Transform::Encrypt(&stored_cipher),
        ))?
    } else {
        metadata.clone()
    };
    Ok(resolve_team_callbacks(&stored, &stored_cipher).map(Answer::Callbacks))
}
