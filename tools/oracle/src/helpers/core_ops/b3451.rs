mod fakes;
mod inputs;
mod operations;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

use inputs::{CredentialInput, FetchInput, FileDataInput, OptionsInput};
use operations::{chunk, init, ocr, upload};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    Init {
        options: OptionsInput,
        #[serde(default)]
        credentials: Vec<CredentialInput>,
    },
    Upload {
        options: OptionsInput,
        #[serde(default)]
        file_data: Option<FileDataInput>,
        #[serde(default)]
        file_url: Option<String>,
        #[serde(default)]
        file_id: Option<String>,
        #[serde(default)]
        fetch: Option<FetchInput>,
    },
    Ocr {
        options: OptionsInput,
        #[serde(default)]
        file_content_base64: Option<String>,
        #[serde(default)]
        content_type: Option<String>,
        #[serde(default)]
        pages: Vec<String>,
    },
    Chunk {
        options: OptionsInput,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        file_content_base64: Option<String>,
        #[serde(default)]
        ocr_was_used: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Init {
                options,
                credentials,
            } => init(options, credentials),
            Input::Upload {
                options,
                file_data,
                file_url,
                file_id,
                fetch,
            } => upload(options, file_data, file_url, file_id, fetch),
            Input::Ocr {
                options,
                file_content_base64,
                content_type,
                pages,
            } => ocr(options, file_content_base64, content_type, pages),
            Input::Chunk {
                options,
                text,
                file_content_base64,
                ocr_was_used,
            } => chunk(options, text, file_content_base64, *ocr_was_used),
        }
    }
}
