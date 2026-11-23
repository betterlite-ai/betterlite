use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;

use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::request::{SelectedTokenizer, TextInput, TokenRequest};
use betterlite_tokenizer::limits::{Models, get_modified_max_tokens};

use super::convert::{ModifiedRequest, TokenCountRequest, read};
use super::error::TokensError;
use super::runtime::TokensRuntime;
use super::store;

fn custom_tokenizer(handle: Option<u64>) -> Result<Option<SelectedTokenizer>, TokensError> {
    match handle {
        None => Ok(None),
        Some(handle) => Ok(Some(SelectedTokenizer::HuggingFace(store::engine(handle)?))),
    }
}

impl TokenCountRequest {
    fn counting<'request>(
        &'request self,
        custom: &'request Option<SelectedTokenizer>,
        text: &'request Option<TextInput>,
    ) -> TokenRequest<'request> {
        TokenRequest {
            model: &self.model,
            custom_tokenizer: custom.as_ref(),
            text: text.as_ref(),
            messages: self.messages.as_deref(),
            count_response_tokens: self.count_response_tokens,
            tools: self.tools.as_deref(),
            tool_choice: self.tool_choice.as_ref(),
        }
    }
}

// LiteLLM joins a list of texts into one string before counting, and the wheel does that join before this call
#[pyfunction]
pub(crate) fn token_count(py: Python<'_>, request: &str) -> PyResult<usize> {
    let wire: TokenCountRequest = read(request)?;
    let text: Option<TextInput> = wire.text.clone().map(TextInput::One);
    let custom = custom_tokenizer(wire.custom_tokenizer)?;
    let counting = wire.counting(&custom, &text);
    let counted = py.detach(|| -> Result<usize, TokensError> {
        let runtime = TokensRuntime::build()?;
        get_runtime()
            .block_on(runtime.counter.count(&counting))
            .map_err(TokensError::from)
    })?;
    Ok(counted)
}

// LiteLLM counts with token_counter(base_model, messages) and leaves its limit alone when that count raises
fn input_tokens(py: Python<'_>, wire: &ModifiedRequest) -> Result<Option<usize>, TokensError> {
    let counting = wire.input_request();
    py.detach(|| -> Result<Option<usize>, TokensError> {
        let runtime = TokensRuntime::build()?;
        Ok(get_runtime()
            .block_on(runtime.counter.count(&counting))
            .ok())
    })
}

#[pyfunction]
pub(crate) fn modified_max_tokens(py: Python<'_>, request: &str) -> PyResult<Option<i64>> {
    let wire: ModifiedRequest = read(request)?;
    let counted = match wire.user_max_tokens {
        None => None,
        Some(_user) => input_tokens(py, &wire)?,
    };
    let catalog = BundledCatalog::default().get();
    let models = Models {
        model: &wire.model,
        base_model: &wire.base_model,
    };
    let modified = get_modified_max_tokens(
        &catalog,
        models,
        wire.user_max_tokens,
        counted,
        &wire.settings(),
    );
    Ok(modified.max_tokens)
}

mod tests;
