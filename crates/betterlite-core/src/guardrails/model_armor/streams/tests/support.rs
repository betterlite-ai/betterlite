use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::model_armor::streams::assembly::Assembly;
use crate::guardrails::model_armor::streams::chunk::{Chunk, field};
use crate::guardrails::model_armor::streams::errors::HttpError;
use crate::guardrails::model_armor::streams::surface::Surface;
use crate::guardrails::model_armor::streams::wire::{OutputParts, ScanFailure, StreamWire};
use crate::guardrails::streaming_rewrite::shapes::RequestData;

#[derive(Debug)]
pub struct FakeFailure;

impl std::fmt::Display for FakeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the fake wiring failed")
    }
}

impl std::error::Error for FakeFailure {}

pub enum ScanReply {
    Armor(GuardrailValue),
    Api(String),
    Unexpected,
}

pub enum EmitReply {
    Chunks(Vec<Chunk>),
    Fail,
}

pub struct Setup {
    pub anthropic: bool,
    pub sse_error: bool,
    pub chat: Option<Assembly>,
    pub anthropic_body: Option<Assembly>,
    pub model_text: Option<String>,
    pub parts: Vec<OutputParts>,
    pub scan: ScanReply,
    pub block: bool,
    pub sanitized: Option<String>,
    pub deidentify: bool,
    pub logging: GuardrailValue,
    pub block_detail: GuardrailValue,
    pub emit: EmitReply,
    pub frames: Vec<Chunk>,
    pub responses: Option<Vec<Chunk>>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            anthropic: false,
            sse_error: false,
            chat: None,
            anthropic_body: None,
            model_text: None,
            parts: Vec::new(),
            scan: ScanReply::Armor(GuardrailValue::Null),
            block: false,
            sanitized: None,
            deidentify: false,
            logging: GuardrailValue::Null,
            block_detail: GuardrailValue::Null,
            emit: EmitReply::Chunks(Vec::new()),
            frames: Vec::new(),
            responses: None,
        }
    }
}

#[derive(Default)]
pub struct Log {
    scans: Mutex<Vec<String>>,
    applied: Mutex<Vec<String>>,
    emitted: Mutex<Vec<GuardrailValue>>,
    frames: Mutex<Vec<String>>,
    block_args: Mutex<Vec<bool>>,
    builders: Mutex<Vec<&'static str>>,
}

pub struct FakeWire {
    pub setup: Setup,
    log: Log,
}

impl FakeWire {
    pub fn new(setup: Setup) -> Self {
        Self {
            setup,
            log: Log::default(),
        }
    }

    pub fn scans(&self) -> Vec<String> {
        let scans = self.log.scans.lock().expect("the fake log lock");
        scans.clone()
    }

    pub fn applied(&self) -> Vec<String> {
        let applied = self.log.applied.lock().expect("the fake log lock");
        applied.clone()
    }

    pub fn emitted(&self) -> Vec<GuardrailValue> {
        let emitted = self.log.emitted.lock().expect("the fake log lock");
        emitted.clone()
    }

    pub fn frame_messages(&self) -> Vec<String> {
        let frames = self.log.frames.lock().expect("the fake log lock");
        frames.clone()
    }

    pub fn block_args(&self) -> Vec<bool> {
        let args = self.log.block_args.lock().expect("the fake log lock");
        args.clone()
    }

    pub fn builders(&self) -> Vec<&'static str> {
        let builders = self.log.builders.lock().expect("the fake log lock");
        builders.clone()
    }
}

impl StreamWire for FakeWire {
    type Failure = FakeFailure;

    fn is_anthropic_sse(&self, _chunks: &[Chunk]) -> bool {
        self.setup.anthropic
    }

    fn is_sse_error(&self, _chunks: &[Chunk]) -> bool {
        self.setup.sse_error
    }

    fn assemble_chat(&self, _chunks: &[Chunk]) -> impl Future<Output = Option<Assembly>> + Send {
        self.log
            .builders
            .lock()
            .expect("the fake log lock")
            .push("chat");
        ready(self.setup.chat.clone())
    }

    fn assemble_anthropic(
        &self,
        _chunks: &[Chunk],
    ) -> impl Future<Output = Option<Assembly>> + Send {
        self.log
            .builders
            .lock()
            .expect("the fake log lock")
            .push("anthropic");
        ready(self.setup.anthropic_body.clone())
    }

    fn model_text(&self, _body: &GuardrailValue) -> Result<String, FakeFailure> {
        match &self.setup.model_text {
            Some(text) => Ok(text.clone()),
            None => Err(FakeFailure),
        }
    }

    fn output_parts(&self, item: &GuardrailValue) -> OutputParts {
        let index = match field(item, "index") {
            Some(GuardrailValue::SignedInteger(index)) => usize::try_from(*index).unwrap_or(0),
            _ => 0,
        };
        self.setup.parts.get(index).cloned().unwrap_or_default()
    }

    fn scan(
        &self,
        content: &str,
    ) -> impl Future<Output = Result<GuardrailValue, ScanFailure<FakeFailure>>> + Send {
        self.log
            .scans
            .lock()
            .expect("the fake log lock")
            .push(content.to_owned());
        let reply = match &self.setup.scan {
            ScanReply::Armor(armor) => Ok(armor.clone()),
            ScanReply::Api(detail) => Err(ScanFailure::Api {
                detail: detail.clone(),
            }),
            ScanReply::Unexpected => Err(ScanFailure::Unexpected(FakeFailure)),
        };
        ready(reply)
    }

    fn should_block(&self, _armor: &GuardrailValue, allow_sanitization: bool) -> bool {
        self.log
            .block_args
            .lock()
            .expect("the fake log lock")
            .push(allow_sanitization);
        self.setup.block
    }

    fn sanitized_content(&self, _armor: &GuardrailValue) -> Option<String> {
        self.setup.sanitized.clone()
    }

    fn has_deidentify(&self, _armor: &GuardrailValue) -> bool {
        self.setup.deidentify
    }

    fn logging_response(&self, _armor: &GuardrailValue) -> GuardrailValue {
        self.setup.logging.clone()
    }

    fn block_detail(&self, _message: &str, _armor: &GuardrailValue) -> GuardrailValue {
        self.setup.block_detail.clone()
    }

    fn emit_sanitized(
        &self,
        _surface: Surface,
        body: &GuardrailValue,
    ) -> Result<Vec<Chunk>, FakeFailure> {
        self.log
            .emitted
            .lock()
            .expect("the fake log lock")
            .push(body.clone());
        match &self.setup.emit {
            EmitReply::Chunks(chunks) => Ok(chunks.clone()),
            EmitReply::Fail => Err(FakeFailure),
        }
    }

    fn anthropic_error_frames(&self, message: &str) -> Result<Vec<Chunk>, FakeFailure> {
        self.log
            .frames
            .lock()
            .expect("the fake log lock")
            .push(message.to_owned());
        Ok(self.setup.frames.clone())
    }

    fn responses_error_items(&self, _error: &HttpError) -> Option<Vec<Chunk>> {
        self.setup.responses.clone()
    }

    fn applied_guardrail(&self, data: &mut RequestData, guardrail_name: &str) {
        self.log
            .applied
            .lock()
            .expect("the fake log lock")
            .push(guardrail_name.to_owned());
        let mut names = match data.remove("applied_guardrails") {
            Some(GuardrailValue::Array(names)) => names,
            _ => Vec::new(),
        };
        names.push(GuardrailValue::Text(guardrail_name.to_owned()));
        data.insert(
            "applied_guardrails".to_owned(),
            GuardrailValue::Array(names),
        );
    }
}

pub fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("the fixture is valid JSON")
}

pub fn chunk(json: &str) -> Chunk {
    Chunk::Json(value(json))
}

pub fn bytes(text: &str) -> Chunk {
    Chunk::Bytes(text.as_bytes().to_vec())
}

pub fn model_body() -> Assembly {
    Assembly::Model(value(r#"{"id": "chatcmpl-1", "choices": []}"#))
}

pub fn frame_text(items: &[Chunk]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            Chunk::Bytes(bytes) => {
                Some(String::from_utf8(bytes.clone()).expect("the frame is utf-8"))
            }
            Chunk::Json(_) => None,
        })
        .collect()
}
