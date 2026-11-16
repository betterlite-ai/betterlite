use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::runway::video_lifecycle::artifact::ArtifactTransport;
use betterlite_core::api::runway::video_lifecycle::content::content_response;
use betterlite_core::api::runway::video_lifecycle::error::VideoLifecycleError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_oracle_helper_api::helper::HelperResult;
use http::HeaderMap;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::{ContentCall, ReplyCall, failure};

const DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
struct Sent {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
}

struct Script {
    reply: (u16, String),
    sent: Mutex<Vec<Sent>>,
}

impl Script {
    fn new(reply: &ReplyCall) -> Self {
        Self {
            reply: (reply.status, reply.body.clone()),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ArtifactTransport for Script {
    fn fetch(
        &self,
        request: OutboundRequest,
        _max_bytes: usize,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let headers: Vec<(String, String)> = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
            })
            .collect();
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method: request.method.as_str().to_owned(),
                url: request.url.to_string(),
                headers,
            });
        ready(Ok(UpstreamResponse {
            status: self.reply.0,
            headers: HeaderMap::new(),
            body: self.reply.1.clone().into_bytes(),
        }))
    }
}

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(present, _)| present == name)
        .map(|(_, value)| value.as_str())
}

fn artifact_value(sent: &[Sent]) -> Value {
    let Some(first) = sent.first() else {
        return json!({
            "method": null,
            "url": null,
            "authorization": null,
            "runway_version": null,
        });
    };
    json!({
        "method": first.method,
        "url": first.url,
        "authorization": header(&first.headers, "authorization"),
        "runway_version": header(&first.headers, "x-runway-version"),
    })
}

fn content_value(outcome: Result<Vec<u8>, VideoLifecycleError>, sent: &[Sent]) -> Value {
    match outcome {
        Ok(bytes) => json!({"content": hex::encode(bytes), "artifact": artifact_value(sent)}),
        Err(VideoLifecycleError::ArtifactStatus(error)) => json!({
            "artifact_status": error.status_code,
            "artifact_body": error.message,
            "artifact": artifact_value(sent),
        }),
        Err(error) => json!({"error": error.to_string()}),
    }
}

pub(super) fn content(calls: &[ContentCall]) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let limits = ApiLimits::default();
    let mut results = Vec::with_capacity(calls.len());
    for call in calls {
        let script = Script::new(&call.reply);
        let outcome = runtime.block_on(content_response(
            call.body.as_bytes(),
            &script,
            limits,
            DEADLINE,
        ));
        results.push(content_value(outcome, &script.requests()));
    }
    Ok(json!({"results": results}))
}
