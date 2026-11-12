use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{
    CertificateRecord, ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::fields::{
    certificate_fields, managed_fields, password_fields, secret_fields, workload_fields,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptStep {
    credential: String,
    outcome: OutcomeName,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum OutcomeName {
    Token,
    Unavailable,
    Authentication,
}

#[derive(Clone)]
struct Outcome {
    name: OutcomeName,
    token: String,
    message: String,
}

pub(super) struct Recorder {
    records: Mutex<Vec<Value>>,
    calls: Mutex<Vec<String>>,
    scripts: BTreeMap<String, Outcome>,
    errors: BTreeMap<String, String>,
}

impl Recorder {
    pub(super) fn new(scripts: &[ScriptStep], errors: &BTreeMap<String, String>) -> Self {
        let scripted = scripts
            .iter()
            .map(|step| {
                (
                    step.credential.clone(),
                    Outcome {
                        name: step.outcome,
                        token: step.token.clone().unwrap_or_default(),
                        message: step.message.clone(),
                    },
                )
            })
            .collect();
        Self {
            records: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            scripts: scripted,
            errors: errors.clone(),
        }
    }

    pub(super) fn records(&self) -> Vec<Value> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, credential: &str, mut recorded: Value) -> Result<(), GrantError> {
        if let Some(message) = self.errors.get(credential) {
            return Err(GrantError {
                credential: "fixture",
                message: message.clone(),
            });
        }
        if let Some(map) = recorded.as_object_mut() {
            map.insert(
                "credential".to_owned(),
                Value::String(credential.to_owned()),
            );
        }
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        Ok(())
    }

    fn outcome(&self, credential: &str) -> Outcome {
        self.scripts
            .get(credential)
            .cloned()
            .unwrap_or_else(|| Outcome {
                name: OutcomeName::Token,
                token: format!("{credential}-token"),
                message: String::new(),
            })
    }
}

pub(crate) struct Scripted<'recorder> {
    recorder: &'recorder Recorder,
    credential: String,
}

impl Grant for Scripted<'_> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        self.recorder
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.credential.clone());
        let outcome = self.recorder.outcome(&self.credential);
        match outcome.name {
            OutcomeName::Token => Ok(ChainToken {
                token: SecretString::from(outcome.token),
                expires_on: None,
                token_type: None,
                refresh_on: None,
            }),
            OutcomeName::Unavailable => Err(LinkError::Unavailable(outcome.message)),
            OutcomeName::Authentication => Err(LinkError::Authentication(outcome.message)),
        }
    }
}

fn managed_name(record: &ManagedIdentityRecord) -> String {
    match record.client_id.as_deref() {
        Some(client_id) if !client_id.is_empty() => "user".to_owned(),
        Some(_) | None => "system".to_owned(),
    }
}

fn grant<'recorder>(
    recorder: &'recorder Recorder,
    credential: &'static str,
    fields: Value,
) -> Result<Scripted<'recorder>, GrantError> {
    recorder.record(credential, fields)?;
    Ok(Scripted {
        recorder,
        credential: credential.to_owned(),
    })
}

impl Grants for Recorder {
    type Grant<'recorder>
        = Scripted<'recorder>
    where
        Self: 'recorder;

    fn client_secret<'recorder>(
        &'recorder self,
        record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'recorder>, GrantError> {
        grant(self, "ClientSecretCredential", secret_fields(record))
    }

    fn certificate<'recorder>(
        &'recorder self,
        record: &CertificateRecord,
    ) -> Result<Self::Grant<'recorder>, GrantError> {
        grant(self, "CertificateCredential", certificate_fields(record))
    }

    fn username_password<'recorder>(
        &'recorder self,
        record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'recorder>, GrantError> {
        grant(self, "UsernamePasswordCredential", password_fields(record))
    }

    fn workload<'recorder>(
        &'recorder self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'recorder>, GrantError> {
        grant(self, "WorkloadIdentityCredential", workload_fields(record))
    }

    fn managed_identity<'recorder>(
        &'recorder self,
        record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'recorder>, GrantError> {
        let name = managed_name(record);
        self.record("ManagedIdentityCredential", managed_fields(record))?;
        Ok(Scripted {
            recorder: self,
            credential: format!("ManagedIdentityCredential:{name}"),
        })
    }
}

#[derive(Clone, Default)]
struct Messages(Arc<Mutex<Vec<String>>>);

struct RawMessage(String);

impl Visit for RawMessage {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            format!("{value:?}")
                .trim_matches('"')
                .clone_into(&mut self.0);
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Messages {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut message = RawMessage(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let messages = Messages::default();
    let subscriber = tracing_subscriber::registry().with(messages.clone());
    let result = tracing::subscriber::with_default(subscriber, run);
    let captured = messages
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (result, captured)
}
