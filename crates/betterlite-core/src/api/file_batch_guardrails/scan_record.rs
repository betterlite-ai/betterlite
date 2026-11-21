use std::collections::BTreeSet;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::call_type::scannable_call_type;
use super::custom_id::custom_id_of;
use super::fingerprint::fingerprint;
use super::hook::{
    GuardrailsOnlyPreCall, PreCallInput, RaisedGuardrail, is_content_block, naming_guardrail,
};
use super::record_json::dumps::{Rendering, dumps};
use super::record_json::text::RecordText;
use super::record_json::{RecordJson, RecordObject};
use super::records::{
    BatchScanFailure, ParsedRecord, RecordDropped, Redaction, ScanOutcome, UnparseableRecord,
    UnroutableRecord, UnscannableRecord,
};
use super::{INJECTED_KEYS, ROUTE_APPLIED_KEY, SCAN_METADATA_BAGS};
use crate::routes::catalog::RouteCatalog;

// What every record of one upload is scanned with
pub struct ScanContext<'scan, Hook> {
    pub scan_metadata: &'scan RecordObject,
    pub user_api_key_dict: &'scan UserAPIKeyAuth,
    pub proxy_logging_obj: &'scan Hook,
    pub routes: &'scan RouteCatalog,
}

// None when the record ships as it was written
pub async fn scan_record<Hook>(
    record: &ParsedRecord,
    context: &ScanContext<'_, Hook>,
) -> Result<Option<ScanOutcome>, Hook::Failure>
where
    Hook: GuardrailsOnlyPreCall,
{
    let line_number = record.line_number;
    let body_key = RecordText::from("body");
    let Some(body) = record
        .payload
        .get(&body_key)
        .and_then(RecordJson::as_object)
    else {
        let failure = BatchScanFailure::Unparseable(UnparseableRecord { line_number });
        return Ok(Some(ScanOutcome::Failure(failure)));
    };
    let custom_id = custom_id_of(&record.payload);
    let url = record.payload.get(&RecordText::from("url"));
    let Some(call_type) = scannable_call_type(context.routes, url, body) else {
        let failure = BatchScanFailure::Unscannable(UnscannableRecord {
            line_number,
            custom_id,
            url: url
                .and_then(RecordJson::as_text)
                .map(|text| text.replaced('?')),
        });
        return Ok(Some(ScanOutcome::Failure(failure)));
    };
    let input = PreCallInput {
        user_api_key_dict: context.user_api_key_dict,
        data: scan_input(body, context.scan_metadata),
        call_type,
        guardrails_only: true,
    };
    let answer = context.proxy_logging_obj.pre_call_hook(input).await;
    judge(record, body, custom_id, answer)
}

// Both bags, deep and per record, because headers and tags would otherwise be shared with the upload and every other record
fn scan_input(body: &RecordObject, scan_metadata: &RecordObject) -> RecordObject {
    let mut data: RecordObject = body
        .iter()
        .filter(|(key, _)| {
            !INJECTED_KEYS
                .iter()
                .any(|name| RecordText::from(*name) == **key)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for bag in SCAN_METADATA_BAGS {
        data.insert(
            RecordText::from(bag),
            RecordJson::Object(scan_metadata.clone()),
        );
    }
    data
}

// A judged block drops the record, and any other failure aborts the scan with its own status
fn judge<Failure>(
    record: &ParsedRecord,
    body: &RecordObject,
    custom_id: Option<String>,
    answer: Result<RecordObject, Failure>,
) -> Result<Option<ScanOutcome>, Failure>
where
    Failure: RaisedGuardrail,
{
    let line_number = record.line_number;
    let scanned = match answer {
        Ok(scanned) => scanned,
        Err(failure) if is_content_block(&failure) => {
            let guardrail = naming_guardrail(&failure);
            let dropped = RecordDropped {
                line_number,
                custom_id,
                guardrail,
            };
            return Ok(Some(ScanOutcome::Dropped(dropped)));
        }
        Err(failure) => return Err(failure),
    };
    if let Some(unroutable) = rerouted(&scanned, line_number, custom_id.as_deref()) {
        let failure = BatchScanFailure::Unroutable(unroutable);
        return Ok(Some(ScanOutcome::Failure(failure)));
    }
    Ok(redaction(record, body, scanned, custom_id).map(ScanOutcome::Redaction))
}

// pre_call_hook marks metadata when a guardrail sent the request to another model, and a batch file goes to one provider
fn rerouted(
    scanned: &RecordObject,
    line_number: usize,
    custom_id: Option<&str>,
) -> Option<UnroutableRecord> {
    let metadata = scanned
        .get(&RecordText::from("metadata"))
        .and_then(RecordJson::as_object)?;
    let Some(RecordJson::Bool(true)) = metadata.get(&RecordText::from(ROUTE_APPLIED_KEY)) else {
        return None;
    };
    let guardrail = metadata
        .get(&RecordText::from("sensitive_data_routing_guardrail"))
        .and_then(RecordJson::as_text)
        .map(|name| name.replaced('?'));
    Some(UnroutableRecord {
        line_number,
        custom_id: custom_id.map(str::to_owned),
        guardrail,
    })
}

// The injected keys are restored from the record, never diffed, so the hook's bookkeeping does not ship upstream
fn redaction(
    record: &ParsedRecord,
    body: &RecordObject,
    mut scanned: RecordObject,
    custom_id: Option<String>,
) -> Option<Redaction> {
    let injected = |key: &RecordText| {
        INJECTED_KEYS
            .iter()
            .any(|name| RecordText::from(*name) == *key)
    };
    let compared: BTreeSet<&RecordText> = body
        .keys()
        .chain(scanned.keys())
        .filter(|key| !injected(*key))
        .collect();
    if fingerprint(&scanned, &compared) == fingerprint(body, &compared) {
        return None;
    }
    scanned.retain(|key, _| !injected(key));
    for name in INJECTED_KEYS {
        let key = RecordText::from(name);
        if let Some(own) = body.get(&key) {
            scanned.insert(key, own.clone());
        }
    }
    let mut payload = record.payload.clone();
    payload.insert(RecordText::from("body"), RecordJson::Object(scanned));
    Some(Redaction {
        line_number: record.line_number,
        custom_id,
        text: dumps(&RecordJson::Object(payload), Rendering::AsRead),
    })
}
