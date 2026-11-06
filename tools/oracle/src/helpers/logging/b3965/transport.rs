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

// The oracle sends no request anywhere: every answer is scripted and every call is recorded for comparison.
pub(super) struct Scripted {
    pub(super) replies: Mutex<VecDeque<HttpResponse>>,
    pub(super) calls: Mutex<Vec<Value>>,
}

struct Call<'call> {
    method: &'call str,
    url: &'call str,
    headers: &'call [Header],
    form: Option<Value>,
    payload: Option<Value>,
}

impl Scripted {
    fn answer(&self, url: &str, call: Value) -> Result<HttpResponse, TransportError> {
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

fn recorded(call: &Call<'_>) -> Value {
    let names: BTreeSet<String> = call
        .headers
        .iter()
        .map(|header| header.name.clone())
        .collect();
    json!({
        "method": call.method,
        "url": call.url,
        "headers": names.into_iter().collect::<Vec<String>>(),
        "form": call.form.clone(),
        "payload": call.payload.clone(),
    })
}

impl Transport for Scripted {
    fn get(&self, url: &str, headers: &[Header]) -> Result<HttpResponse, TransportError> {
        let call = recorded(&Call {
            method: "GET",
            url,
            headers,
            form: None,
            payload: None,
        });
        self.answer(url, call)
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[Header],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        let fields: BTreeMap<String, String> = form
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let call = recorded(&Call {
            method: "POST",
            url,
            headers,
            form: Some(json!(fields)),
            payload: None,
        });
        self.answer(url, call)
    }

    fn post_json(
        &self,
        url: &str,
        headers: &[Header],
        payload: &IndexMap<String, Json>,
    ) -> Result<HttpResponse, TransportError> {
        let call = recorded(&Call {
            method: "POST",
            url,
            headers,
            form: None,
            payload: Some(payload_view(payload)),
        });
        self.answer(url, call)
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
