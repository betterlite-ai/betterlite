#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::RawValue;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::presidio_actions::request_data::RequestData;
use crate::presidio_actions::{AnonymizerSettings, PresidioAnonymizer};
use crate::presidio_analysis::wire::{MockRedactedItem, MockRedactedText};
use crate::presidio_analysis::{AnalyzerSettings, DEFAULT_CHUNK_SIZE_BYTES, PresidioAnalyzer};
use crate::presidio_response::HookContext;
use crate::presidio_response::wire::{ChatMessage, Content, ContentItem, ResponseBody};

pub(super) static NO_ENTITIES: BTreeMap<String, PiiAction> = BTreeMap::new();
pub(super) static NO_PARAMS: BTreeMap<String, GuardrailValue> = BTreeMap::new();

pub(super) fn request_data(json: &str) -> RequestData {
    serde_json::from_str(json).expect("request data")
}

pub(super) fn response_body(json: &str) -> ResponseBody {
    let raw: Box<RawValue> = serde_json::from_str(json).expect("response JSON");
    ResponseBody::from_raw(raw).expect("response body")
}

pub(super) fn analyzer(api_base: &str, masked: Option<&str>) -> PresidioAnalyzer {
    PresidioAnalyzer {
        settings: AnalyzerSettings {
            guardrail_name: "presidio-global".to_owned(),
            analyzer_api_base: format!("{api_base}/"),
            language: "en".to_owned(),
            ad_hoc_recognizers: None,
            pii_entities: Vec::new(),
            output_parse_pii: false,
            apply_to_output: false,
            score_thresholds: BTreeMap::new(),
            entities_deny_list: Vec::new(),
            chunk_size_bytes: DEFAULT_CHUNK_SIZE_BYTES,
            timeout: Duration::from_secs(5),
            mock_redacted_text: masked.map(mock_text),
        },
        client: reqwest::Client::default(),
        chunk_semaphores: Mutex::default(),
    }
}

fn mock_text(text: &str) -> MockRedactedText {
    MockRedactedText {
        text: text.to_owned(),
        items: vec![MockRedactedItem {
            entity_type: Some("PERSON".to_owned()),
        }],
    }
}

pub(super) fn anonymizer(api_base: &str) -> PresidioAnonymizer {
    PresidioAnonymizer {
        settings: AnonymizerSettings {
            anonymizer_api_base: format!("{api_base}/"),
            timeout: Duration::from_secs(5),
        },
        client: reqwest::Client::default(),
        clock: fixed_clock,
    }
}

fn fixed_clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("fixed clock")
}

pub(super) fn context<'context>(
    analyzer: &'context PresidioAnalyzer,
    anonymizer: &'context PresidioAnonymizer,
) -> HookContext<'context> {
    HookContext {
        analyzer,
        anonymizer,
        pii_entities_config: &NO_ENTITIES,
        dynamic_params: &NO_PARAMS,
        global_output_parse_pii: false,
    }
}

pub(super) fn messages(request: &RequestData) -> Vec<ChatMessage> {
    let raw = request.get("messages").expect("messages field");
    serde_json::from_str(raw.get()).expect("messages JSON")
}

pub(super) fn content_text(content: Option<&Content>) -> Option<String> {
    match content? {
        Content::Text(text) => Some(text.clone()),
        Content::Blocks(_) | Content::Other(_) => None,
    }
}

pub(super) fn chat_content(response: &ResponseBody) -> Option<String> {
    let ResponseBody::Chat(chat) = response else {
        return None;
    };
    let choice = chat.choices.as_ref()?.first()?;
    let message = choice.message.as_ref()?;
    content_text(message.content.as_ref())
}

pub(super) fn anthropic_texts(response: &ResponseBody) -> Vec<String> {
    let ResponseBody::Anthropic(message) = response else {
        return Vec::new();
    };
    message
        .content
        .iter()
        .flatten()
        .filter_map(|item| match item {
            ContentItem::Object(part) if part.kind.as_deref() == Some("text") => part.text.clone(),
            ContentItem::Object(_) | ContentItem::Other(_) => None,
        })
        .collect()
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, DefaultGuard) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Sink(Arc::clone(&captured));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(move || sink.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn logged_text(captured: &Mutex<Vec<u8>>) -> String {
    let bytes = captured
        .lock()
        .map_or_else(|_poisoned| Vec::new(), |log| log.clone());
    String::from_utf8(bytes).expect("captured log is UTF-8")
}

pub(super) fn logged_count(request: &RequestData) -> usize {
    let Some(bucket) = request
        .get("litellm_metadata")
        .or_else(|| request.get("metadata"))
    else {
        return 0;
    };
    let Ok(metadata) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(bucket.get()) else {
        return 0;
    };
    let Some(entries) = metadata.get("standard_logging_guardrail_information") else {
        return 0;
    };
    serde_json::from_str::<Vec<Box<RawValue>>>(entries.get()).map_or(0, |logged| logged.len())
}
