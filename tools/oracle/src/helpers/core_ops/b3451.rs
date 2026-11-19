mod fakes;
mod inputs;
mod operations;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

use inputs::{ChunkInput, InitInput, OcrInput, UploadInput};
use operations::{init, ocr, upload};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    Init(InitInput),
    Upload(UploadInput),
    Ocr(OcrInput),
    Chunk(ChunkInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Init(fields) => init(fields),
            Input::Upload(fields) => upload(fields),
            Input::Ocr(fields) => ocr(fields),
            Input::Chunk(fields) => Ok(json!({ "chunks": fields.chunks()? })),
        }
    }
}
