use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::json;
use url::Url;

use crate::api::runway::speech::dispatch::{
    DispatchCall, DispatchEnvironment, HandlerFailure, HandlerRequest, TextToSpeechHandler,
    dispatch_text_to_speech,
};
use crate::api::runway::speech::error::RunwaySpeechError;
use crate::api::runway::speech::mapping::VOICE_KEY;
use crate::api::runway::speech::response::RunwaySpeechTransform;
use crate::api::runway::tasks::poll::{PollClock, TaskTransport};
use crate::api::runway::tasks::settings::RunwaySettings;

pub(super) fn settings(pairs: &[(&str, &str)]) -> RunwaySettings {
    let lookup: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RunwaySettings::load(&lookup).unwrap()
}

pub(super) fn text_params(pairs: &[(&str, &str)]) -> Object {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), json!(value)))
        .collect()
}

pub(super) fn request_headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn module_globals(api_base: Option<&str>, api_key: Option<&str>) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        openai_key: None,
        organization: None,
    }
}

pub(super) fn preset_object(preset: &str) -> Object {
    let mut fields = Object::new();
    fields.insert(String::from("type"), json!("runway-preset"));
    fields.insert(String::from("presetId"), json!(preset));
    fields
}

pub(super) fn voice_params(preset: &str) -> Object {
    let mut params = Object::new();
    params.insert(VOICE_KEY.to_owned(), json!(preset_object(preset)));
    params
}

pub(super) fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn submission(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) headers: Vec<(&'static str, &'static str)>,
}

impl Reply {
    pub(super) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    pub(super) fn audio(content_type: &'static str, body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_owned(),
            headers: vec![("content-type", content_type)],
        }
    }
}

pub(super) struct Script {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<(String, HeaderMap)>>,
}

impl Script {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<(String, HeaderMap)> {
        self.requests.lock().unwrap().clone()
    }
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap()
            .push((url.to_string(), headers));
        let reply = self.replies.lock().unwrap().pop_front().unwrap_or(Reply {
            status: 500,
            body: String::new(),
            headers: Vec::new(),
        });
        let reply_headers: HeaderMap = reply
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    HeaderName::from_static(name),
                    HeaderValue::from_static(value),
                )
            })
            .collect();
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: reply_headers,
            body: reply.body.into_bytes(),
        }))
    }
}

pub(super) struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<u64>>,
}

impl Clock {
    pub(super) fn new(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
            pauses: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn pauses(&self) -> Vec<u64> {
        self.pauses.lock().unwrap().clone()
    }
}

impl PollClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses.lock().unwrap().push(duration.as_secs());
        ready(())
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) seen: Vec<Object>,
    pub(super) failure: Option<HandlerFailure>,
}

impl TextToSpeechHandler for Recorder {
    type Output = Object;

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<Object, HandlerFailure>> + Send {
        let mut record = Object::new();
        record.insert(String::from("model"), json!(request.model));
        record.insert(String::from("input"), json!(request.input));
        record.insert(String::from("voice"), json!(request.voice));
        record.insert(
            String::from("optional_params"),
            json!(request.optional_params),
        );
        record.insert(
            String::from("litellm_params"),
            json!(request.litellm_params),
        );
        record.insert(String::from("api_base"), json!(request.api_base));
        record.insert(
            String::from("api_key"),
            json!(request.api_key.map(secrecy::ExposeSecret::expose_secret)),
        );
        record.insert(
            String::from("timeout_seconds"),
            json!(request.timeout_seconds),
        );
        record.insert(String::from("extra_headers"), json!(request.extra_headers));
        record.insert(String::from("aspeech"), json!(request.aspeech));
        self.seen.push(record.clone());
        let answer: Result<Object, HandlerFailure> = self
            .failure
            .clone()
            .map_or(Ok(record), |failure| Err(failure));
        ready(answer)
    }
}

#[derive(Default)]
pub(super) struct Run {
    pub(super) voice: Option<SpeechVoice>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) timeout_seconds: f64,
    pub(super) extra_headers: Option<Object>,
    pub(super) aspeech: bool,
    pub(super) failure: Option<HandlerFailure>,
}

impl Run {
    pub(super) fn with_voice(mut self, voice: SpeechVoice) -> Self {
        self.voice = Some(voice);
        self
    }

    pub(super) fn with_optional(mut self, optional_params: Object) -> Self {
        self.optional_params = optional_params;
        self
    }

    pub(super) fn with_params(mut self, litellm_params: Object) -> Self {
        self.litellm_params = litellm_params;
        self
    }

    pub(super) fn with_explicit(mut self, base: Option<&str>, key: Option<&str>) -> Self {
        self.api_base = base.map(str::to_owned);
        self.api_key = key.map(str::to_owned);
        self
    }

    pub(super) fn with_timeout(mut self, timeout_seconds: f64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    pub(super) fn with_headers(mut self, extra_headers: Object) -> Self {
        self.extra_headers = Some(extra_headers);
        self
    }

    pub(super) fn speaking(mut self) -> Self {
        self.aspeech = true;
        self
    }

    pub(super) fn failing(mut self, failure: HandlerFailure) -> Self {
        self.failure = Some(failure);
        self
    }
}

pub(super) struct Dispatched {
    pub(super) answer: Result<Object, RunwaySpeechError>,
    pub(super) seen: Vec<Object>,
}

pub(super) async fn dispatch_run(
    loaded: &RunwaySettings,
    globals: &LiteLlmGlobals,
    run: &Run,
) -> Dispatched {
    let environment = DispatchEnvironment {
        globals,
        settings: loaded,
    };
    let mut recorder = Recorder {
        seen: Vec::new(),
        failure: run.failure.clone(),
    };
    let answer = dispatch_text_to_speech(
        DispatchCall {
            model: "eleven_multilingual_v2",
            input: "hello",
            voice: run.voice.as_ref(),
            optional_params: &run.optional_params,
            litellm_params: run.litellm_params.clone(),
            api_base: run.api_base.clone(),
            api_key: run
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.clone())),
            timeout_seconds: run.timeout_seconds,
            extra_headers: run.extra_headers.as_ref(),
            aspeech: run.aspeech,
            environment: &environment,
        },
        &mut recorder,
    )
    .await;
    Dispatched {
        answer,
        seen: recorder.seen,
    }
}

pub(super) async fn fetch(
    loaded: &RunwaySettings,
    replies: &[Reply],
    submission: &UpstreamResponse,
    headers: &IndexMap<String, String>,
) -> (
    Result<BinaryContent, RunwaySpeechError>,
    Vec<(String, HeaderMap)>,
    Vec<u64>,
) {
    let script = Script::new(replies);
    let clock = Clock::new(0);
    let answer = RunwaySpeechTransform {
        transport: &script,
        clock: &clock,
        settings: loaded,
    }
    .transform(submission, headers)
    .await;
    (answer, script.requests(), clock.pauses())
}
