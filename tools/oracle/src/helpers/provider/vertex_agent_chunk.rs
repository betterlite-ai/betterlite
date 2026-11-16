use betterlite_models::decisions::Extra;
use betterlite_vertex::vertex_agent_stream::parse_chunk;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    chunks: Vec<Extra>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let chunks = input
            .chunks
            .iter()
            .map(project)
            .collect::<Result<Vec<_>, Box<HelperError>>>()?;
        Ok(json!({ "chunks": chunks }))
    }
}

// the four fields chunk_parser puts on a chunk, before the provider identity is added
fn project(fields: &Extra) -> Result<Value, Box<HelperError>> {
    let chunk = parse_chunk(fields).map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "content": chunk.content,
        "role": chunk.role,
        "finish_reason": chunk.finish_reason,
        "usage": chunk.usage.map(|usage| [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens]),
    }))
}

fn failure(message: String) -> Box<HelperError> {
    let error = HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}
