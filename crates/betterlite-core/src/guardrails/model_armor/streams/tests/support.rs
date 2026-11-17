use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrails::model_armor::streams::assembly::Assembly;
use crate::guardrails::model_armor::streams::chunk::{Chunk, field};
use crate::guardrails::model_armor::streams::errors::HttpError;
use crate::guardrails::model_armor::streams::surface::Surface;
use crate::guardrails::model_armor::streams::wire::{OutputParts, ScanFailure, StreamWire};
use crate::guardrails::streaming_rewrite::shapes::RequestData;

#[derive(Debug, thiserror::Error)]
#[error("the fake wiring failed")]
pub(crate) struct FakeFailure;

pub(crate) enum ScanReply {
    Armor(GuardrailValue),
    Api(String),
    Unexpected,
}

pub(crate) enum EmitReply {
    Chunks(Vec<Chunk>),
    Fail,
}

// The two classifiers the fake wire answers with.
#[derive(Default)]
pub(crate) struct WireFlags {
    pub(crate) anthropic: bool,
    pub(crate) sse_error: bool,
}

// The verdict the fake scan reaches on the content it was handed.
#[derive(Default)]
pub(crate) struct Verdict {
    pub(crate) block: bool,
    pub(crate) deidentify: bool,
}

pub(crate) struct Setup {
    pub(crate) wire: WireFlags,
    pub(crate) chat: Option<Assembly>,
    pub(crate) anthropic_body: Option<Assembly>,
    pub(crate) model_text: Option<String>,
    pub(crate) parts: Vec<OutputParts>,
    pub(crate) scan: ScanReply,
    pub(crate) verdict: Verdict,
    pub(crate) sanitized: Option<String>,
    pub(crate) logging: GuardrailValue,
    pub(crate) block_detail: GuardrailValue,
    pub(crate) emit: EmitReply,
    pub(crate) frames: Vec<Chunk>,
    pub(crate) responses: Option<Vec<Chunk>>,
}

pub(crate) fn fixture() -> Setup {
    Setup::default()
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            wire: WireFlags::default(),
            chat: None,
            anthropic_body: None,
            model_text: None,
            parts: Vec::new(),
            scan: ScanReply::Armor(GuardrailValue::Null),
            verdict: Verdict::default(),
            sanitized: None,
            logging: GuardrailValue::Null,
            block_detail: GuardrailValue::Null,
            emit: EmitReply::Chunks(Vec::new()),
            frames: Vec::new(),
            responses: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct Log {
    scans: Mutex<Vec<String>>,
    applied: Mutex<Vec<String>>,
    emitted: Mutex<Vec<GuardrailValue>>,
    frames: Mutex<Vec<String>>,
    block_args: Mutex<Vec<bool>>,
    builders: Mutex<Vec<&'static str>>,
}

pub(crate) struct FakeWire {
    pub(crate) setup: Setup,
    pub(crate) log: Log,
}

impl FakeWire {
    pub(crate) fn new(setup: Setup) -> Self {
        Self {
            setup,
            log: Log::default(),
        }
    }

    pub(crate) fn scans(&self) -> Vec<String> {
        let scans = self.log.scans.lock().expect("the fake log lock");
        scans.clone()
    }

    pub(crate) fn applied(&self) -> Vec<String> {
        let applied = self.log.applied.lock().expect("the fake log lock");
        applied.clone()
    }

    pub(crate) fn emitted(&self) -> Vec<GuardrailValue> {
        let emitted = self.log.emitted.lock().expect("the fake log lock");
        emitted.clone()
    }

    pub(crate) fn frame_messages(&self) -> Vec<String> {
        let frames = self.log.frames.lock().expect("the fake log lock");
        frames.clone()
    }

    pub(crate) fn block_args(&self) -> Vec<bool> {
        let args = self.log.block_args.lock().expect("the fake log lock");
        args.clone()
    }

    pub(crate) fn builders(&self) -> Vec<&'static str> {
        let builders = self.log.builders.lock().expect("the fake log lock");
        builders.clone()
    }

    fn assemble_named(
        &self,
        builder: &'static str,
        body: Option<&Assembly>,
    ) -> impl Future<Output = Option<Assembly>> + Send {
        self.log
            .builders
            .lock()
            .expect("the fake log lock")
            .push(builder);
        ready(body.cloned())
    }
}

impl StreamWire for FakeWire {
    type Failure = FakeFailure;

    fn is_anthropic_sse(&self, _chunks: &[Chunk]) -> bool {
        self.setup.wire.anthropic
    }

    fn is_sse_error(&self, _chunks: &[Chunk]) -> bool {
        self.setup.wire.sse_error
    }

    fn assemble_chat(&self, _chunks: &[Chunk]) -> impl Future<Output = Option<Assembly>> + Send {
        self.assemble_named("chat", self.setup.chat.as_ref())
    }

    fn assemble_anthropic(
        &self,
        _chunks: &[Chunk],
    ) -> impl Future<Output = Option<Assembly>> + Send {
        self.assemble_named("anthropic", self.setup.anthropic_body.as_ref())
    }

    fn model_text(&self, _body: &GuardrailValue) -> Result<String, FakeFailure> {
        self.setup
            .model_text
            .as_ref()
            .map_or(Err(FakeFailure), |text| Ok(text.clone()))
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
        self.setup.verdict.block
    }

    fn sanitized_content(&self, _armor: &GuardrailValue) -> Option<String> {
        self.setup.sanitized.clone()
    }

    fn has_deidentify(&self, _armor: &GuardrailValue) -> bool {
        self.setup.verdict.deidentify
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
            .unwrap_or_else(PoisonError::into_inner)
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
            .unwrap_or_else(PoisonError::into_inner)
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

pub(crate) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).expect("the fixture is valid JSON")
}

pub(crate) fn chunk(json: &str) -> Chunk {
    Chunk::Json(value(json))
}

pub(crate) fn bytes(text: &str) -> Chunk {
    Chunk::Bytes(text.as_bytes().to_vec())
}

pub(crate) fn model_body() -> Assembly {
    Assembly::Model(value(r#"{"id": "chatcmpl-1", "choices": []}"#))
}

pub(crate) fn frame_text(items: &[Chunk]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            Chunk::Bytes(raw) => Some(String::from_utf8(raw.clone()).expect("the frame is utf-8")),
            Chunk::Json(_) => None,
        })
        .collect()
}
