use std::collections::BTreeMap;
use std::fmt::Debug;
use std::panic::Location;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_reporting::bug_report::notice::NOTICE_PREFIX;
use betterlite_server::api::proxy_endpoint_support::testing::ScriptedCatalog;
use betterlite_server::api::proxy_endpoint_support::{
    CaughtException, FailureFacts, HttpFailure, ProxyDiagnostics, handle_exception_on_proxy,
    is_known_model,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber, subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Record {
        exception: ExceptionSpec,
        litellm_call_id: Option<String>,
        notice: bool,
    },
    KnownModel {
        models: Option<Vec<String>>,
        lookups: Vec<Option<String>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ExceptionSpec {
    Http {
        status_code: u16,
        detail: String,
    },
    Proxy {
        message: String,
        error_type: String,
        param: Option<String>,
        code: Option<i64>,
        headers: BTreeMap<String, String>,
    },
    Failure {
        message: String,
        status_code: Option<u16>,
        param: Option<String>,
    },
}

// B4170 owns should_report_bug and its environment flag; the case scripts that answer so this adapter asks only what the case says
struct ScriptedDiagnostics {
    notice: bool,
}

impl ProxyDiagnostics for ScriptedDiagnostics {
    fn bug_report_notice(&self, _failure: &FailureFacts<'_>) -> Option<String> {
        self.notice
            .then(|| format!("{NOTICE_PREFIX} scripted notice"))
    }
}

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn caught_exception(spec: &ExceptionSpec) -> CaughtException<'_> {
    match spec {
        ExceptionSpec::Http {
            status_code,
            detail,
        } => CaughtException::Http(HttpFailure {
            status_code: *status_code,
            detail: detail.clone(),
        }),
        ExceptionSpec::Proxy {
            message,
            error_type,
            param,
            code,
            headers,
        } => CaughtException::Proxy(ProxyException::new(ProxyExceptionInit {
            message: message.clone(),
            error_type: error_type.clone(),
            param: param.clone(),
            code: code.map_or(ExceptionCode::Absent, ExceptionCode::Status),
            headers: headers.clone(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        })),
        ExceptionSpec::Failure {
            message,
            status_code,
            param,
        } => CaughtException::Failure(FailureFacts {
            message,
            exception_type: "Failure",
            status_code: *status_code,
            param: param.as_deref(),
            database_error: None,
            // a tools path, so the record keeps no frame, as a LiteLLM error that was never raised keeps none
            location: Location::caller(),
        }),
    }
}

fn rendered_exception(spec: &ExceptionSpec) -> String {
    match spec {
        ExceptionSpec::Http {
            status_code,
            detail,
        } => format!("{status_code}: {detail}"),
        ExceptionSpec::Proxy { message, .. } | ExceptionSpec::Failure { message, .. } => {
            message.clone()
        }
    }
}

fn record_view(record: &ProxyException) -> Value {
    json!({
        "message": record.message,
        "type": record.error_type,
        "param": record.param,
        "code": record.code,
        "headers": record.headers,
    })
}

fn record_answer(spec: &ExceptionSpec, litellm_call_id: Option<&str>, notice: bool) -> Value {
    let lines = Lines::default();
    let guard = subscriber::set_default(tracing_subscriber::registry().with(lines.clone()));
    let diagnostics = ScriptedDiagnostics { notice };
    let record =
        handle_exception_on_proxy(caught_exception(spec), litellm_call_id, &diagnostics, None);
    let captured = lines
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    drop(guard);
    let exception_line = format!("Exception: {}", rendered_exception(spec));
    json!({
        "record": record_view(&record),
        "exception_logged": captured.contains(&exception_line),
        "notice_logged": captured
            .iter()
            .any(|line| line.starts_with(NOTICE_PREFIX)),
    })
}

fn known_model_answer(
    models: Option<&[String]>,
    lookups: &[Option<String>],
) -> Result<Value, Box<HelperError>> {
    let Some(names) = models else {
        let known = lookups
            .iter()
            .map(|model| is_known_model(model.as_deref(), None))
            .collect::<Result<Vec<bool>, _>>()
            .or_value_error()?;
        return Ok(json!(known));
    };
    let catalog = ScriptedCatalog {
        names: names.to_vec(),
        calls: AtomicUsize::new(0),
    };
    let known = lookups
        .iter()
        .map(|model| is_known_model(model.as_deref(), Some(&catalog)))
        .collect::<Result<Vec<bool>, _>>()
        .or_value_error()?;
    Ok(json!(known))
}

fn answer(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Record {
            exception,
            litellm_call_id,
            notice,
        } => Ok(record_answer(
            exception,
            litellm_call_id.as_deref(),
            *notice,
        )),
        Call::KnownModel { models, lookups } => known_model_answer(models.as_deref(), lookups),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(answer)
            .collect::<Result<Vec<Value>, _>>()?;
        Ok(json!({ "calls": answers }))
    }
}
