use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_logging::galileo::delivery::headers::Header;
use betterlite_logging::galileo::delivery::transport::{
    HttpResponse, TraceIds, Transport, TransportError,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

type Reply = Result<HttpResponse, TransportError>;

// The oracle sends no request anywhere: every answer is scripted and every call is recorded for comparison.
pub(super) struct Scripted {
    pub(super) replies: Mutex<VecDeque<HttpResponse>>,
    pub(super) calls: Mutex<Vec<Value>>,
}

impl Scripted {
    fn answer(&self, url: &str, call: Value) -> Reply {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| TransportError {
                url: url.to_owned(),
                reason: "no scripted reply".to_owned(),
            })
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn call_record(
    method: &str,
    url: &str,
    headers: &[Header],
    form: Option<&Value>,
    payload: Option<&Value>,
) -> Value {
    let names: BTreeSet<String> = headers.iter().map(|header| header.name.clone()).collect();
    json!({
        "method": method,
        "url": url,
        "headers": names.into_iter().collect::<Vec<String>>(),
        "form": form,
        "payload": payload,
    })
}

impl Transport for Scripted {
    fn get(&self, url: &str, headers: &[Header]) -> Reply {
        self.answer(url, call_record("GET", url, headers, None, None))
    }

    fn post_form(&self, url: &str, headers: &[Header], form: &[(&str, &str)]) -> Reply {
        let fields: BTreeMap<&str, &str> = form.iter().copied().collect();
        self.answer(
            url,
            call_record("POST", url, headers, Some(&json!(fields)), None),
        )
    }

    fn post_json(&self, url: &str, headers: &[Header], payload: &IndexMap<String, Json>) -> Reply {
        let view = call_record("POST", url, headers, None, Some(&payload_view(payload)));
        self.answer(url, view)
    }
}

fn payload_view(payload: &IndexMap<String, Json>) -> Value {
    let mut whole = json!(payload);
    let traces = whole
        .as_object_mut()
        .and_then(|fields| fields.remove("traces"))
        .unwrap_or(Value::Null);
    let trace_ids: Vec<Value> = traces
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|trace| json!([trace.pointer("/id"), trace.pointer("/spans/0/id")]))
                .collect()
        })
        .unwrap_or_default();
    json!({"trace_ids": trace_ids, "rest": whole})
}

pub(super) struct CountedIds(pub(super) AtomicUsize);

impl TraceIds for CountedIds {
    fn next_id(&self) -> String {
        let number = self.0.fetch_add(1, Ordering::SeqCst);
        format!("00000000-0000-0000-0000-{number:012x}")
    }
}
