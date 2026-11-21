use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::file_batch_guardrails::call_type::scannable_call_type;
use betterlite_core::api::file_batch_guardrails::custom_id::{custom_id_of, describe};
use betterlite_core::api::file_batch_guardrails::fingerprint::{build_scan_metadata, fingerprint};
use betterlite_core::api::file_batch_guardrails::hook::{
    RaisedGuardrail, is_content_block, naming_guardrail,
};
use betterlite_core::api::file_batch_guardrails::lines::iter_records;
use betterlite_core::api::file_batch_guardrails::record_json::dumps::{Rendering, dumps};
use betterlite_core::api::file_batch_guardrails::record_json::text::RecordText;
use betterlite_core::api::file_batch_guardrails::record_json::{RecordJson, RecordObject};
use betterlite_core::api::file_batch_guardrails::records::{
    BatchRejection, BatchScanFailure, BatchScanResult, RecordChange, RecordDropped, RecordRedacted,
    UnparseableRecord, UnroutableRecord, UnscannableRecord,
};
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};
use betterlite_guardrails::guardrail_base::failure::{
    GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Raise {
    GuardrailRaised,
    BlockedPii,
    ModifyResponse,
    SensitiveDataRoute,
    Http,
    Other,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Unparseable,
    Unscannable,
    Unroutable,
    NothingToSubmit,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Redacted,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Cause {
    Uncaused,
    Explicit,
    Implicit,
}

// One flat shape for a raised exception, a scan failure or a record change; each kind reads its own fields
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Item<Kind> {
    kind: Kind,
    line_number: usize,
    custom_id: Option<String>,
    url: Option<String>,
    guardrail_name: Option<String>,
    detail_guardrail_name: Option<String>,
    blocked_content: bool,
    status: u16,
    cause: Cause,
    offset: u64,
    length: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    CallType {
        url: Json,
        body: IndexMap<String, Json>,
    },
    ScanMetadata {
        metadata: IndexMap<String, Json>,
    },
    CustomId {
        line: String,
    },
    Fingerprint {
        left: String,
        right: String,
    },
    ContentBlock {
        raised: Item<Raise>,
    },
    Report {
        scanned_records: usize,
        changes: Vec<Item<Action>>,
    },
    PublicFailure {
        failure: Item<Failure>,
    },
}

// What Python keeps on the exception object besides B5471's fields: an explicit __cause__ and a guardrail_name attribute
struct Raised<'item> {
    failure: GuardrailFailure,
    item: &'item Item<Raise>,
}

impl RaisedGuardrail for Raised<'_> {
    fn blocked_content(&self) -> Option<bool> {
        let GuardrailFailure::GuardrailRaised {
            blocked_content, ..
        } = &self.failure
        else {
            return None;
        };
        Some(*blocked_content)
    }

    fn has_cause(&self) -> bool {
        self.item.cause == Cause::Explicit
    }

    fn is_intervention(&self) -> bool {
        self.failure.is_intervention()
    }

    // A plain exception carries no guardrail_name attribute at all
    fn guardrail_name(&self) -> Option<&str> {
        let named = self.item.guardrail_name.as_deref();
        named.filter(|_| self.item.kind != Raise::Other)
    }

    fn detail_guardrail_name(&self) -> Option<&str> {
        let GuardrailFailure::HttpObject { detail, .. } = &self.failure else {
            return None;
        };
        let Some(GuardrailValue::Text(name)) = detail.get("guardrail_name") else {
            return None;
        };
        Some(name)
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
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!(answers))
    }
}

fn answer(call: &Call) -> HelperResult {
    match call {
        Call::CallType { url, body } => call_type(url, body),
        Call::ScanMetadata { metadata } => {
            let narrowed = RecordJson::Object(build_scan_metadata(metadata));
            serde_json::from_str(&dumps(&narrowed, Rendering::AsRead)).or_value_error()
        }
        Call::CustomId { line } => {
            let [payload] = payloads(line)?;
            let custom_id = custom_id_of(&payload);
            let described = describe(custom_id.as_deref());
            Ok(json!({"custom_id": custom_id, "described": described}))
        }
        Call::Fingerprint { left, right } => {
            let [left_payload, right_payload] = payloads(&format!("{left}\n{right}\n"))?;
            let keys: BTreeSet<&RecordText> =
                left_payload.keys().chain(right_payload.keys()).collect();
            let same = fingerprint(&left_payload, &keys) == fingerprint(&right_payload, &keys);
            Ok(json!({"same": same}))
        }
        Call::ContentBlock { raised } => {
            let pair = Raised {
                failure: failure(raised),
                item: raised,
            };
            let blocked = is_content_block(&pair);
            Ok(json!({"blocked": blocked, "guardrail": naming_guardrail(&pair)}))
        }
        Call::Report {
            scanned_records,
            changes,
        } => report(*scanned_records, changes),
        Call::PublicFailure { failure } => {
            let error = rejection(failure).to_string();
            Ok(json!({"status": BatchRejection::STATUS, "detail": {"error": error}}))
        }
    }
}

fn call_type(url: &Json, body: &IndexMap<String, Json>) -> HelperResult {
    let routes = RouteCatalog::parse(CATALOG).or_value_error()?;
    let members: RecordObject = body
        .iter()
        .map(|(key, value)| (RecordText::from(key.as_str()), RecordJson::from(value)))
        .collect();
    let found = scannable_call_type(&routes, Some(&RecordJson::from(url)), &members);
    serde_json::to_value(found).or_value_error()
}

fn report(scanned_records: usize, changes: &[Item<Action>]) -> HelperResult {
    let result = BatchScanResult {
        changes: changes.iter().map(change).collect(),
        scanned_records,
        redactions: (),
    };
    let report = serde_json::to_value(result.report()).or_value_error()?;
    let submitted = result.submitted_records();
    Ok(json!({"submitted_records": submitted, "report": report, "summary": result.summary()}))
}

// The records _iter_records reads from these lines, which must hold exactly as many as asked for
fn payloads<const COUNT: usize>(lines: &str) -> Result<[RecordObject; COUNT], Box<HelperError>> {
    let read: Vec<RecordObject> = iter_records(lines.as_bytes())
        .map(|record| record.map(|parsed| parsed.payload))
        .collect::<Result<_, _>>()
        .or_value_error()?;
    <[RecordObject; COUNT]>::try_from(read)
        .map_err(|found| format!("expected {COUNT} records, read {}", found.len()))
        .or_value_error()
}

// The exception LiteLLM's classifier sees, built with the defaults its constructors use
fn failure(raised: &Item<Raise>) -> GuardrailFailure {
    let guardrail_name = raised.guardrail_name.clone();
    match raised.kind {
        Raise::GuardrailRaised => GuardrailFailure::GuardrailRaised {
            guardrail_name,
            message: String::new(),
            wrap_default_message: true,
            status_code: 400,
            blocked_content: raised.blocked_content,
        },
        Raise::BlockedPii => GuardrailFailure::BlockedPii {
            entity_type: "EMAIL_ADDRESS".to_owned(),
            guardrail_name,
            status_code: 400,
        },
        Raise::ModifyResponse => {
            GuardrailFailure::ModifyResponse(Box::new(ModifyResponseFailure {
                message: "blocked".to_owned(),
                model: "m".to_owned(),
                request_data: BTreeMap::new(),
                guardrail_name,
                detection_info: BTreeMap::new(),
                original_response: None,
            }))
        }
        Raise::SensitiveDataRoute => {
            GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
                route_to_model: "local".to_owned(),
                session_id: "s".to_owned(),
                guardrail_name,
                detection_info: BTreeMap::new(),
                message: None,
                sticky_session_routing: false,
            }))
        }
        Raise::Http => http_failure(raised),
        Raise::Other => GuardrailFailure::Other("not a guardrail failure".to_owned()),
    }
}

// An HTTPException's detail is a dict when the guardrail recorded its name there
fn http_failure(raised: &Item<Raise>) -> GuardrailFailure {
    let status = raised.status;
    let Some(name) = &raised.detail_guardrail_name else {
        let detail = "blocked".to_owned();
        return GuardrailFailure::Http { status, detail };
    };
    let detail = IndexMap::from([
        (
            "error".to_owned(),
            GuardrailValue::Text("blocked".to_owned()),
        ),
        (
            "guardrail_name".to_owned(),
            GuardrailValue::Text(name.clone()),
        ),
    ]);
    GuardrailFailure::HttpObject { status, detail }
}

fn change(item: &Item<Action>) -> RecordChange {
    let line_number = item.line_number;
    let custom_id = item.custom_id.clone();
    match item.kind {
        Action::Redacted => RecordChange::Redacted(RecordRedacted {
            line_number,
            custom_id,
            offset: item.offset,
            length: item.length,
        }),
        Action::Dropped => RecordChange::Dropped(RecordDropped {
            line_number,
            custom_id,
            guardrail: item.guardrail_name.clone(),
        }),
    }
}

fn rejection(item: &Item<Failure>) -> BatchRejection {
    let line_number = item.line_number;
    let custom_id = item.custom_id.clone();
    let failure = match item.kind {
        Failure::Unparseable => BatchScanFailure::Unparseable(UnparseableRecord { line_number }),
        Failure::Unscannable => BatchScanFailure::Unscannable(UnscannableRecord {
            line_number,
            custom_id,
            url: item.url.clone(),
        }),
        Failure::Unroutable => BatchScanFailure::Unroutable(UnroutableRecord {
            line_number,
            custom_id,
            guardrail: item.guardrail_name.clone(),
        }),
        Failure::NothingToSubmit => return BatchRejection::NothingToSubmit,
    };
    BatchRejection::Failure(failure)
}
