use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{PromptIgnoreFlags, PromptParams};
use secrecy::{ExposeSecret as _, SecretString};

use crate::humanloop::logger::{ChatCompletionPromptRequest, HumanloopLogger};
use crate::humanloop::prompt_cache::{Clock, PromptCache};
use crate::humanloop::prompt_manager::HumanLoopPromptManager;
use crate::humanloop::transport::{PromptFetch, PromptTransport, TransportResponse};

pub(super) const PROMPT_BODY: &str = r#"{
    "id": "pr_abc123",
    "model": "gpt-4o",
    "temperature": 0.2,
    "max_tokens": 64,
    "endpoint": "chat",
    "template": [
        {"role": "system", "content": "You help {{name}}."},
        {"role": "user", "content": "Say hello"}
    ]
}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedFetch {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
}

pub(super) type Recorded = Arc<Mutex<Vec<RecordedFetch>>>;

pub(super) struct FakeTransport {
    pub(super) status: Option<u16>,
    pub(super) body: String,
    pub(super) recorded: Recorded,
}

impl PromptTransport for FakeTransport {
    type Error = io::Error;

    fn get(&self, fetch: &PromptFetch) -> Result<TransportResponse, io::Error> {
        let headers = fetch
            .headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.expose_secret().to_owned()))
            .collect();
        let mut recorded = self
            .recorded
            .lock()
            .map_err(|_poisoned| io::Error::other("recorder lock poisoned"))?;
        recorded.push(RecordedFetch {
            url: fetch.url.clone(),
            headers,
        });
        drop(recorded);
        self.status.map_or_else(
            || Err(io::Error::other("connection reset")),
            |status| {
                Ok(TransportResponse {
                    status,
                    body: self.body.clone(),
                })
            },
        )
    }
}

#[derive(Clone, Debug)]
pub(super) struct FakeClock(pub(super) Arc<Mutex<Duration>>);

impl FakeClock {
    pub(super) fn advance(&self, seconds: u64) {
        let mut now = self.0.lock().expect("clock lock");
        *now = now
            .checked_add(Duration::from_secs(seconds))
            .expect("fake clock does not overflow");
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.0.lock().expect("clock lock")
    }
}

pub(super) fn fake_clock() -> FakeClock {
    FakeClock(Arc::new(Mutex::new(Duration::from_secs(1_000))))
}

pub(super) fn replying(status: u16, body: &str) -> (FakeTransport, Recorded) {
    let recorded = Recorded::default();
    let transport = FakeTransport {
        status: Some(status),
        body: body.to_owned(),
        recorded: Arc::clone(&recorded),
    };
    (transport, recorded)
}

pub(super) fn manager(
    transport: FakeTransport,
    clock: &FakeClock,
    ttl_seconds: u64,
) -> HumanLoopPromptManager<FakeTransport, FakeClock> {
    HumanLoopPromptManager {
        transport,
        cache: PromptCache {
            clock: clock.clone(),
            ttl: Duration::from_secs(ttl_seconds),
            entries: HashMap::new(),
        },
    }
}

pub(super) fn logger(
    transport: FakeTransport,
    clock: &FakeClock,
    api_key: Option<&str>,
) -> HumanloopLogger<FakeTransport, FakeClock> {
    HumanloopLogger {
        api_key: api_key.map(SecretString::from),
        manager: Mutex::new(manager(transport, clock, 60)),
    }
}

pub(super) fn fill_cache(
    prompts: &mut HumanLoopPromptManager<FakeTransport, FakeClock>,
    key: &SecretString,
    count: usize,
    advance_seconds: Option<u64>,
) {
    for index in 0..count {
        prompts
            .get_prompt_from_id(&format!("pr_{index}"), key)
            .expect("fill");
        if let Some(seconds) = advance_seconds {
            prompts.cache.clock.advance(seconds);
        }
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture json")
}

pub(super) fn params(text: &str) -> PromptParams {
    serde_json::from_str(text).expect("fixture params")
}

pub(super) fn request<'request>(
    prompt_id: Option<&'request str>,
    prompt_variables: Option<&'request PromptParams>,
    dynamic_callback_params: &'request Json,
) -> ChatCompletionPromptRequest<'request> {
    ChatCompletionPromptRequest {
        model: "humanloop/gpt-3.5",
        messages: vec![json(r#"{"role": "user", "content": "caller message"}"#)],
        non_default_params: params(r#"{"temperature": 0.9, "top_p": 0.5}"#),
        prompt_id,
        prompt_variables,
        dynamic_callback_params,
        ignore_flags: PromptIgnoreFlags::default(),
    }
}
