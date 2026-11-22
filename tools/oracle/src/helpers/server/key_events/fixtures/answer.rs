use std::sync::Arc;

use betterlite_config::sections::key_event_sinks::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secret_managers::secret_contracts::options::SecretTags;
use betterlite_server::audit_events::callbacks::AuditCallbackConfig;
use betterlite_server::audit_events::identity::CallerIdentity;
use betterlite_server::audit_events::settings::AuditSettings;
use betterlite_server::audit_events::sink::AuditClock;
use betterlite_server::key_event_sinks::ports::TeamDirectory;
use betterlite_server::key_event_sinks::secrets::SecretSink;
use betterlite_server::key_events::KeyEvents;
use betterlite_server::key_events::audit::KeyEventAudit;
use betterlite_server::key_events::effects::KeyEventEffects;
use betterlite_server::key_events::types::{
    DeletedEvent, GeneratedEvent, KeyEvent, KeyEventCaller, RotatedEvent, UpdatedEvent,
};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::regenerate::RegenerateKeyRequest;
use betterlite_server::key_fields::request::GenerateKeyRequest;
use betterlite_server::key_fields::response::GenerateKeyResponse;
use betterlite_server::key_fields::update::UpdateKeyRequest;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio_util::task::TaskTracker;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::types::{Caller, EventKind, Input, Runtime, failure};
use super::{Board, Levels, RecordingScheduler, ScriptedManager, ScriptedTeams, WaitingEmail};

// litellm.store_audit_logs with the environment unset and the OSS proxy's premium_user false
struct EnabledAudit {
    enabled: bool,
}

impl AuditSettings for EnabledAudit {
    fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        explicit.unwrap_or(self.enabled)
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &[]
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

struct FixedClock(DateTime<FixedOffset>);

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

pub(crate) async fn answer(input: &Input) -> HelperResult {
    let board = Arc::new(Board::default());
    let _logging = set_default(tracing_subscriber::registry().with(Levels {
        board: Arc::clone(&board),
    }));
    let event = event(input)?;
    let settings = settings(input);
    let manager = ScriptedManager {
        board: Arc::clone(&board),
        secrets: input.secrets.clone(),
        failing: input.failing.clone(),
    };
    let teams = ScriptedTeams {
        board: Arc::clone(&board),
        metadata: input.teams.clone(),
        database: input.runtime == Runtime::Ready,
    };
    let directory: Option<&dyn TeamDirectory> = match input.runtime {
        Runtime::Absent => None,
        Runtime::NoDatabase | Runtime::Ready => Some(&teams),
    };
    let email = WaitingEmail {
        alerting: input.alerting.clone(),
        board: Arc::clone(&board),
        tasks: TaskTracker::new(),
    };
    let effects = KeyEventEffects {
        secrets: SecretSink {
            settings: &settings,
            manager: input.client.then_some(&manager),
            teams: directory,
        },
        email: &email,
    };
    let audit = KeyEventAudit {
        settings: Arc::new(EnabledAudit {
            enabled: input.audit == Some(true),
        }),
        clock: Arc::new(audit_clock(input)?),
        new_id: {
            let id = input.id.clone();
            Arc::new(move || id.clone())
        },
        scheduler: Arc::new(RecordingScheduler {
            board: Arc::clone(&board),
        }),
        litellm_proxy_admin_name: input.admin_name.clone(),
    };
    let caller = key_event_caller(&input.caller);
    let raised = KeyEvents {
        audit: &audit,
        effects,
    }
    .dispatch(&event, &caller, input.changed_by.as_deref())
    .await
    .is_err();
    Ok(json!({"raised": raised, "trace": board.entries()}))
}

fn settings(input: &Input) -> Settings {
    Settings {
        store_virtual_keys: input.settings.store_virtual_keys == Some(true),
        prefix_for_stored_virtual_keys: input.settings.prefix_for_stored_virtual_keys.clone(),
        description: input.settings.description.clone(),
        tags: input.settings.tags.clone().map(SecretTags::Mapping),
        alerting: input.alerting.clone(),
    }
}

fn audit_clock(input: &Input) -> Result<FixedClock, Box<HelperError>> {
    let moment =
        DateTime::parse_from_rfc3339(&input.now).map_err(|error| failure("InvalidInput", error))?;
    Ok(FixedClock(moment))
}

fn key_event_caller(call: &Caller) -> KeyEventCaller {
    KeyEventCaller {
        identity: CallerIdentity {
            user_id: call.user_id.clone(),
            api_key: call.api_key.clone().map(SecretString::from),
            metadata: call.metadata.clone(),
            team_metadata: call.team_metadata.clone(),
        },
        token: call.token.clone().map(SecretString::from),
    }
}

fn event(input: &Input) -> Result<KeyEvent, Box<HelperError>> {
    match input.event {
        EventKind::Generated => generated_event(input),
        EventKind::Updated => updated_event(input),
        EventKind::Rotated => rotated_event(input),
        EventKind::Deleted => deleted_event(input),
    }
}

fn generated_event(input: &Input) -> Result<KeyEvent, Box<HelperError>> {
    let data = GenerateKeyRequest::read(&decoded::<TypedValue>(input.data.as_ref(), "data")?)
        .map_err(|error| failure("ValidationError", error))?;
    let response =
        GenerateKeyResponse::read(&decoded::<TypedValue>(input.response.as_ref(), "response")?)
            .map_err(|error| failure("ValidationError", error))?;
    Ok(KeyEvent::Generated(GeneratedEvent { data, response }))
}

fn updated_event(input: &Input) -> Result<KeyEvent, Box<HelperError>> {
    let data = UpdateKeyRequest::read(&decoded::<TypedValue>(input.data.as_ref(), "data")?)
        .map_err(|error| failure("ValidationError", error))?;
    let row = decoded::<LiteLLMVerificationToken>(input.row.as_ref(), "row")?;
    let response = decoded::<JsonData>(input.response.as_ref(), "response")?;
    Ok(KeyEvent::Updated(UpdatedEvent {
        data,
        row,
        response,
    }))
}

fn rotated_event(input: &Input) -> Result<KeyEvent, Box<HelperError>> {
    let data = match input.data.as_ref() {
        Some(value) => Some(
            RegenerateKeyRequest::read(&decoded::<TypedValue>(Some(value), "data")?)
                .map_err(|error| failure("ValidationError", error))?,
        ),
        None => None,
    };
    let row = decoded::<LiteLLMVerificationToken>(input.row.as_ref(), "row")?;
    let response =
        GenerateKeyResponse::read(&decoded::<TypedValue>(input.response.as_ref(), "response")?)
            .map_err(|error| failure("ValidationError", error))?;
    Ok(KeyEvent::Rotated(RotatedEvent {
        data,
        row,
        response,
    }))
}

fn deleted_event(input: &Input) -> Result<KeyEvent, Box<HelperError>> {
    let request = decoded::<JsonData>(input.data.as_ref(), "data")?;
    let mut rows = Vec::with_capacity(input.rows.len());
    for value in &input.rows {
        rows.push(decoded::<JsonData>(Some(value), "rows")?);
    }
    let response = decoded::<JsonData>(input.response.as_ref(), "response")?;
    Ok(KeyEvent::Deleted(DeletedEvent {
        request,
        rows,
        response,
    }))
}

fn decoded<T>(value: Option<&JsonValue>, name: &str) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let Some(value) = value else {
        return Err(failure(
            "InvalidInput",
            format!("the {name} field is required"),
        ));
    };
    serde_json::from_value(value.clone()).map_err(|error| failure("InvalidInput", error))
}
