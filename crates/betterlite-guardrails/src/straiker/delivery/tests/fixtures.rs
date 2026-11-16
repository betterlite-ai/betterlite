#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::{Future, ready};
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::delivery::blocked_turns::BlockedTurns;
use crate::straiker::delivery::retry::RetryDelay;
use crate::straiker::delivery::settings::StraikerSettings;
use crate::straiker::delivery::transport::{WebhookPost, WebhookReply, WebhookRequest};
use crate::straiker::delivery::{ApplyRequest, DeliveryCall, StraikerDelivery};
use crate::straiker::v3::answer_shape::AnthropicTranslator;
use crate::straiker::v3::prompt_text::PromptTokenDecoder;
use crate::straiker_webhook::context::ProviderNames;
use crate::straiker_webhook::failure::WebhookFailure;
use crate::third_party_init_b::straiker::StraikerArgs;

pub(super) const V1_ALLOW: &str = r#"{"action":"NONE"}"#;
pub(super) const V1_BLOCK: &str =
    r#"{"action":"BLOCKED","blocked_reason":"policy","blocked_by":["pii"],"turnId":"turn-1"}"#;
pub(super) const V1_INTERVENE: &str = r#"{"action":"GUARDRAIL_INTERVENED","texts":["rewritten"]}"#;
pub(super) const V3_ALLOW: &str = r#"{"straiker":{"action":"allow"}}"#;
pub(super) const V3_BLOCK: &str =
    r#"{"straiker":{"action":"block","blocked_by":["pii"],"block_message":"policy"}}"#;
pub(super) const V3_BLOCK_STATE: &str =
    r#"{"straiker":{"action":"block","block_message":"kill switch"}}"#;

pub(super) fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn object(source: &str) -> Object {
    match parsed(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

pub(super) fn raw() -> StraikerArgs {
    StraikerArgs {
        name: "straiker".to_owned(),
        api_key: SecretString::from("sk_live_test"),
        api_base: "https://api.straiker.test/".to_owned(),
        source: "LiteLLM Gateway".to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        timeout: None,
        max_retries: None,
        initial_backoff: None,
        max_backoff: None,
        unreachable_fallback: "fail_closed".to_owned(),
        fail_on_error: true,
        max_payload_bytes: None,
        custom_headers: None,
        metadata: None,
        verbose: None,
        api_version: None,
        agent_ref: None,
        client: None,
        format_hint: None,
    }
}

pub(super) fn v3_raw() -> StraikerArgs {
    let mut args = raw();
    args.api_key = SecretString::from("sk_agt_test");
    args
}

pub(super) fn settings(raw: &StraikerArgs) -> StraikerSettings {
    StraikerSettings::resolve(raw).unwrap()
}

pub(super) fn request_data(source: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(source).unwrap()
}

pub(super) fn api_inputs(source: &str) -> GuardrailApiInputs {
    serde_json::from_str(source).unwrap()
}

pub(super) fn inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(super) fn reply(status: u16, body: &str) -> Result<WebhookReply, WebhookFailure> {
    Ok(WebhookReply {
        status,
        body: body.to_owned(),
    })
}

pub(super) fn network_error() -> Result<WebhookReply, WebhookFailure> {
    Err(WebhookFailure {
        message: "ConnectError: refused".to_owned(),
        is_unreachable: true,
        retryable: true,
    })
}

pub(super) fn permanent_error() -> Result<WebhookReply, WebhookFailure> {
    Err(WebhookFailure {
        message: "decode error".to_owned(),
        is_unreachable: false,
        retryable: false,
    })
}

#[derive(Clone, Debug)]
pub(super) struct Sent {
    pub(super) url: String,
    pub(super) body: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) authorization: String,
    pub(super) timeout: Duration,
}

pub(super) struct FakePost {
    pub(super) replies: Mutex<VecDeque<Result<WebhookReply, WebhookFailure>>>,
    pub(super) sent: Mutex<Vec<Sent>>,
}

impl WebhookPost for FakePost {
    fn post(
        &self,
        request: WebhookRequest<'_>,
    ) -> impl Future<Output = Result<WebhookReply, WebhookFailure>> + Send {
        let sent = Sent {
            url: request.url.to_owned(),
            body: request.body.to_owned(),
            headers: request.headers.values.clone(),
            authorization: request.headers.authorization.expose_secret().to_owned(),
            timeout: request.timeout,
        };
        self.sent.lock().unwrap().push(sent);
        let scheduled = self.replies.lock().unwrap().pop_front().unwrap_or_else(|| {
            Err(WebhookFailure {
                message: "no scripted reply".to_owned(),
                is_unreachable: true,
                retryable: false,
            })
        });
        ready(scheduled)
    }
}

#[derive(Default)]
pub(super) struct FakeDelay {
    pub(super) caps: Mutex<Vec<Duration>>,
}

impl RetryDelay for FakeDelay {
    fn delay(&self, cap: Duration) -> Duration {
        self.caps.lock().unwrap().push(cap);
        Duration::ZERO
    }
}

#[derive(Clone, Default)]
pub(super) struct Clock {
    seconds: Arc<Mutex<f64>>,
}

impl Clock {
    pub(super) fn source(&self) -> Arc<dyn Fn() -> f64 + Send + Sync> {
        let seconds = Arc::clone(&self.seconds);
        Arc::new(move || *seconds.lock().unwrap())
    }

    pub(super) fn advance(&self, by: f64) {
        let mut seconds = self.seconds.lock().unwrap();
        *seconds = by.mul_add(1.0_f64, *seconds);
    }
}

pub(super) fn delivery(
    settings: StraikerSettings,
    replies: Vec<Result<WebhookReply, WebhookFailure>>,
) -> StraikerDelivery<FakePost, FakeDelay> {
    delivery_at(settings, replies, &Clock::default())
}

pub(super) fn delivery_at(
    settings: StraikerSettings,
    replies: Vec<Result<WebhookReply, WebhookFailure>>,
    clock: &Clock,
) -> StraikerDelivery<FakePost, FakeDelay> {
    StraikerDelivery {
        settings,
        transport: FakePost {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        },
        delay: FakeDelay::default(),
        blocked_turns: BlockedTurns::new(clock.source()).unwrap(),
    }
}

pub(super) fn sent(delivery: &StraikerDelivery<FakePost, FakeDelay>) -> Vec<Sent> {
    delivery.transport.sent.lock().unwrap().clone()
}

pub(super) fn caps<Post>(delivery: &StraikerDelivery<Post, FakeDelay>) -> Vec<Duration> {
    delivery.delay.caps.lock().unwrap().clone()
}

#[derive(Default)]
pub(super) struct Providers;

impl ProviderNames for Providers {
    fn provider_for(
        &self,
        _model: &str,
        _api_base: Option<&str>,
        _api_key: Option<&str>,
    ) -> Option<String> {
        None
    }
}

#[derive(Default)]
pub(super) struct Routes;

impl RouteCallTypes<CallType> for Routes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<CallType>> {
        match route {
            "/v1/chat/completions" => Some(vec![CallType::Completion]),
            "/v1/messages" => Some(vec![CallType::AnthropicMessages]),
            "/v1/completions" => Some(vec![CallType::TextCompletion]),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct NoTokens;

impl PromptTokenDecoder for NoTokens {
    fn decode(&self, _ids: &[u32]) -> Option<String> {
        None
    }
}

#[derive(Default)]
pub(super) struct Translator;

impl AnthropicTranslator for Translator {
    fn translate(&self, response: &Object) -> Object {
        response.clone()
    }
}

#[derive(Default)]
pub(super) struct Fakes {
    pub(super) providers: Providers,
    pub(super) routes: Routes,
    pub(super) decoder: NoTokens,
    pub(super) translator: Translator,
}

impl Fakes {
    pub(super) fn call<'call>(
        &'call self,
        call_type: Option<&'call str>,
        call_id: Option<&'call str>,
    ) -> DeliveryCall<'call> {
        DeliveryCall {
            call_type,
            litellm_call_id: call_id,
            litellm_trace_id: None,
            provider_names: &self.providers,
            routes: &self.routes,
            mappings: &[],
            decoder: &self.decoder,
            translator: &self.translator,
        }
    }
}

pub(super) async fn run<Post, Delay>(
    delivery: &StraikerDelivery<Post, Delay>,
    data: &mut BTreeMap<String, GuardrailValue>,
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
    call: &DeliveryCall<'_>,
) -> Result<GuardrailApiInputs, GuardrailFailure>
where
    Post: WebhookPost,
    Delay: RetryDelay,
{
    let mut request = ApplyRequest {
        inputs,
        request_data: data,
        input_type,
        call,
    };
    delivery.apply(&mut request).await
}

#[derive(Clone, Default)]
pub(super) struct Fields(Arc<Mutex<Vec<(String, String)>>>);

impl Fields {
    pub(super) fn take(&self) -> Vec<(String, String)> {
        mem::take(&mut self.0.lock().unwrap())
    }
}

impl<S: Subscriber> Layer<S> for Fields {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = EveryField::default();
        event.record(&mut fields);
        self.0.lock().unwrap().extend(fields.0);
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(super) fn take(&self) -> Vec<String> {
        mem::take(&mut self.0.lock().unwrap())
    }
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if !matches!(*event.metadata().level(), Level::WARN | Level::ERROR) {
            return;
        }
        let mut message = Messages::default();
        event.record(&mut message);
        self.0.lock().unwrap().extend(message.0);
    }
}

#[derive(Default)]
struct EveryField(Vec<(String, String)>);

impl Visit for EveryField {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name().to_owned(), value.to_owned()));
    }
}

#[derive(Default)]
struct Messages(Vec<String>);

impl Visit for Messages {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }
}

pub(super) fn capture_fields() -> (Fields, DefaultGuard) {
    let fields = Fields::default();
    let guard = set_default(tracing_subscriber::registry().with(fields.clone()));
    (fields, guard)
}

pub(super) fn capture_warnings() -> (Warnings, DefaultGuard) {
    let warnings = Warnings::default();
    let guard = set_default(tracing_subscriber::registry().with(warnings.clone()));
    (warnings, guard)
}
