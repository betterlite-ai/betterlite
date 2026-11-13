use std::io::{Result as IoResult, Write};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::ser::Formatter;

use super::error::XaiBatchesError;
use super::records::{XaiBatchResult, XaiResultCode};

#[derive(Serialize)]
struct OutputError {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct OutputResponse {
    status_code: u16,
    request_id: Option<Json>,
    body: IndexMap<String, Json>,
}

#[derive(Serialize)]
struct OutputLine {
    id: String,
    custom_id: String,
    response: Option<OutputResponse>,
    error: Option<OutputError>,
}

fn output_line(result: &XaiBatchResult) -> OutputLine {
    let id = format!("batch_req_{}", result.batch_request_id);
    let failure = result.batch_result.error.as_ref();
    let first_value = result
        .batch_result
        .response
        .as_ref()
        .and_then(|response| response.values().next());
    let Some(body) = first_value else {
        let message = failure.map_or_else(
            || "xAI returned no response for this request".to_owned(),
            |reported| reported.message.clone(),
        );
        let code = failure
            .and_then(|reported| reported.code.as_ref())
            .map_or_else(|| "request_failed".to_owned(), code_text);
        return OutputLine {
            id,
            custom_id: result.batch_request_id.clone(),
            response: None,
            error: Some(OutputError { code, message }),
        };
    };
    OutputLine {
        id,
        custom_id: result.batch_request_id.clone(),
        response: Some(OutputResponse {
            status_code: 200,
            request_id: body.get("id").cloned(),
            body: body.clone(),
        }),
        error: None,
    }
}

fn code_text(code: &XaiResultCode) -> String {
    match code {
        XaiResultCode::Integer(number) => number.to_string(),
        XaiResultCode::Text(text) => text.clone(),
    }
}

// json.dumps(..., ensure_ascii=False) writes ", " and ": " between items
struct PythonSeparators;

impl Formatter for PythonSeparators {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> IoResult<()>
    where
        W: ?Sized + Write,
    {
        writer.write_all(if first { b"" } else { b", " })
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> IoResult<()>
    where
        W: ?Sized + Write,
    {
        writer.write_all(if first { b"" } else { b", " })
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> IoResult<()>
    where
        W: ?Sized + Write,
    {
        writer.write_all(b": ")
    }
}

pub fn results_to_openai_jsonl(results: &[XaiBatchResult]) -> Result<Vec<u8>, XaiBatchesError> {
    let mut output = Vec::new();
    for result in results {
        let mut line = Vec::new();
        let mut serializer = serde_json::Serializer::with_formatter(&mut line, PythonSeparators);
        output_line(result)
            .serialize(&mut serializer)
            .map_err(|source| XaiBatchesError::Encode { source })?;
        line.push(b'\n');
        output.extend(line);
    }
    Ok(output)
}
