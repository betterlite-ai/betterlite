use std::num::NonZeroUsize;

use betterlite_tokenizer::count::settings::Settings;
use betterlite_tokenizer::count::wire::TokenRequestWire;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

mod fixture;

use fixture::fixture;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    TokenCounter,
    DefaultEncoding,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    function: Function,
    #[serde(flatten)]
    request: TokenRequestWire,
    #[serde(default)]
    disable_token_counter: bool,
    #[serde(default)]
    max_exact_chars: Option<usize>,
    #[serde(default)]
    chunk_size_chars: Option<usize>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        runtime.block_on(count(input))
    }
}

async fn count(input: &Input) -> HelperResult {
    let directory = tempfile::tempdir().map_err(|error| failure(error.to_string()))?;
    let built = fixture(settings(input)?, directory.path())?;
    match input.function {
        Function::TokenCounter => {
            let selected = input
                .request
                .selected()
                .map_err(|error| failure(error.to_string()))?;
            let counted = built
                .counter
                .count(&input.request.request(selected.as_ref()))
                .await
                .map_err(|error| failure(error.to_string()))?;
            Ok(json!(counted))
        }
        Function::DefaultEncoding => {
            let Some(text) = input.request.text.as_ref() else {
                return Err(failure("text is required".to_owned()));
            };
            let engine = built
                .default
                .get()
                .await
                .map_err(|error| failure(error.to_string()))?;
            let joined = text.joined();
            let ids = engine
                .encode(joined.as_ref())
                .map_err(|error| failure(error.to_string()))?;
            Ok(json!(ids.len()))
        }
    }
}

fn settings(input: &Input) -> Result<Settings, Box<HelperError>> {
    let base = Settings::default();
    Ok(Settings {
        disabled: input.disable_token_counter,
        max_exact_chars: positive(
            input.max_exact_chars,
            base.max_exact_chars,
            "max_exact_chars",
        )?,
        chunk_chars: positive(input.chunk_size_chars, base.chunk_chars, "chunk_size_chars")?,
    })
}

fn positive(
    value: Option<usize>,
    fallback: NonZeroUsize,
    name: &str,
) -> Result<NonZeroUsize, Box<HelperError>> {
    value.map_or(Ok(fallback), |number| {
        NonZeroUsize::new(number).ok_or_else(|| failure(format!("{name} must be positive")))
    })
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
