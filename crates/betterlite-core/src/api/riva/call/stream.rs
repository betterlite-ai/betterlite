use std::time::Duration;

use pyo3::exceptions::{PyAttributeError, PyTypeError, PyValueError};
use pyo3::types::{PyAnyMethods as _, PyBytes, PyDict, PyDictMethods as _, PyList};
use pyo3::{Bound, PyAny, PyResult, Python};

use super::error::RivaCallError;
use super::sdk::{Prepared, provider_call, provider_failure};
use crate::api::riva::records::response::{FinalResult, WordTiming};

// 1600 int16 samples, the source's streaming slice size; its 50 ms comment is really 100 ms
pub const CHUNK_BYTES: usize = 3_200;

const RESULTS: &str = "results";
const IS_FINAL: &str = "is_final";
const ALTERNATIVES: &str = "alternatives";
const TRANSCRIPT: &str = "transcript";
const WORDS: &str = "words";
const WORD: &str = "word";
const START_TIME: &str = "start_time";
const END_TIME: &str = "end_time";
const GENERATOR: &str = "streaming_response_generator";
const AUDIO_CHUNKS: &str = "audio_chunks";
const STREAMING_CONFIG: &str = "streaming_config";
const TIMEOUT: &str = "timeout";

// One protobuf field read as the source's getattr with a default does
type Found<'py> = Result<Option<Bound<'py, PyAny>>, RivaCallError>;

// Everything the source's try block wraps: the service, the generator, its deadline and the stream
pub fn stream_attached(
    py: Python<'_>,
    prepared: &Prepared,
    pcm: &[u8],
    timeout: Duration,
) -> Result<Vec<FinalResult>, RivaCallError> {
    let service = prepared.sdk.asr_service(py, &prepared.auth)?;
    let generator = provider_call(py, service.bind(py).getattr(GENERATOR))?;
    let keywords = PyDict::new(py);
    provider_call(py, keywords.set_item(AUDIO_CHUNKS, chunk_list(py, pcm)?))?;
    provider_call(
        py,
        keywords.set_item(STREAMING_CONFIG, prepared.streaming.bind(py)),
    )?;
    if supports_timeout(py, &generator)? {
        provider_call(py, keywords.set_item(TIMEOUT, timeout.as_secs_f64()))?;
    }
    let stream = provider_call(py, generator.call((), Some(&keywords)))?;
    collect(py, &stream)
}

fn chunk_list<'py>(py: Python<'py>, pcm: &[u8]) -> Result<Bound<'py, PyList>, RivaCallError> {
    let mut chunks = Vec::new();
    for chunk in pcm.chunks(CHUNK_BYTES) {
        chunks.push(PyBytes::new(py, chunk));
    }
    provider_call(py, PyList::new(py, chunks))
}

// The source forwards the deadline only when the generator declares it or takes keyword variadics
fn supports_timeout(py: Python<'_>, generator: &Bound<'_, PyAny>) -> Result<bool, RivaCallError> {
    let inspect = provider_call(py, py.import("inspect"))?;
    let signature = match inspect.call_method1("signature", (generator,)) {
        Ok(signature) => signature,
        Err(error)
            if error.is_instance_of::<PyTypeError>(py)
                || error.is_instance_of::<PyValueError>(py) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(provider_failure(py, &error)),
    };
    let parameters = provider_call(py, signature.getattr("parameters"))?;
    if provider_call(py, parameters.contains(TIMEOUT))? {
        return Ok(true);
    }
    keyword_variadic(py, &inspect, &parameters)
}

fn keyword_variadic(
    py: Python<'_>,
    inspect: &Bound<'_, PyAny>,
    parameters: &Bound<'_, PyAny>,
) -> Result<bool, RivaCallError> {
    let parameter_class = provider_call(py, inspect.getattr("Parameter"))?;
    let variadic = provider_call(py, parameter_class.getattr("VAR_KEYWORD"))?;
    let values = provider_call(py, parameters.call_method0("values"))?;
    for entry in items(py, &values)? {
        let kind = entry
            .getattr("kind")
            .map_err(|error| provider_failure(py, &error))?;
        if provider_call(py, kind.eq(&variadic))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn collect(py: Python<'_>, stream: &Bound<'_, PyAny>) -> Result<Vec<FinalResult>, RivaCallError> {
    let mut finals = Vec::new();
    let responses = provider_call(py, stream.try_iter())?;
    for response in responses {
        let response = response.map_err(|error| provider_failure(py, &error))?;
        collect_response(py, &response, &mut finals)?;
    }
    Ok(finals)
}

fn collect_response(
    py: Python<'_>,
    response: &Bound<'_, PyAny>,
    finals: &mut Vec<FinalResult>,
) -> Result<(), RivaCallError> {
    let Some(results) = present(py, response, RESULTS)? else {
        return Ok(());
    };
    for result in items(py, &results)? {
        if present(py, &result, IS_FINAL)?.is_none() {
            continue;
        }
        let Some(alternatives) = present(py, &result, ALTERNATIVES)? else {
            continue;
        };
        let top = provider_call(py, alternatives.get_item(0))?;
        let transcript = transcript(py, &top)?;
        let words = words(py, &top)?;
        finals.push(FinalResult { transcript, words });
    }
    Ok(())
}

// getattr(object, name, None): an absent attribute reads as None, any other error is the provider's
fn skip_missing<'py>(py: Python<'py>, result: PyResult<Bound<'py, PyAny>>) -> Found<'py> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_instance_of::<PyAttributeError>(py) => Ok(None),
        Err(error) => Err(provider_failure(py, &error)),
    }
}

// getattr(object, name, None) or []: a missing, None or falsey value yields no items
fn present<'py>(py: Python<'py>, object: &Bound<'py, PyAny>, name: &str) -> Found<'py> {
    let Some(value) = skip_missing(py, object.getattr(name))? else {
        return Ok(None);
    };
    let truthy = provider_call(py, value.is_truthy())?;
    Ok(truthy.then_some(value))
}

fn items<'py>(
    py: Python<'py>,
    container: &Bound<'py, PyAny>,
) -> Result<Vec<Bound<'py, PyAny>>, RivaCallError> {
    let mut values = Vec::new();
    let iterator = provider_call(py, container.try_iter())?;
    for value in iterator {
        values.push(value.map_err(|error| provider_failure(py, &error))?);
    }
    Ok(values)
}

fn transcript(py: Python<'_>, top: &Bound<'_, PyAny>) -> Result<String, RivaCallError> {
    match present(py, top, TRANSCRIPT)? {
        Some(value) => text_value(&value),
        None => Ok(String::new()),
    }
}

fn text_value(value: &Bound<'_, PyAny>) -> Result<String, RivaCallError> {
    match value.extract::<String>() {
        Ok(text) => Ok(text),
        Err(_error) => Err(RivaCallError::Malformed {
            message: String::from("a transcript is not a string"),
        }),
    }
}

fn words(py: Python<'_>, top: &Bound<'_, PyAny>) -> Result<Vec<WordTiming>, RivaCallError> {
    let Some(container) = present(py, top, WORDS)? else {
        return Ok(Vec::new());
    };
    let mut timings = Vec::new();
    for word in items(py, &container)? {
        timings.push(word_timing(py, &word)?);
    }
    Ok(timings)
}

fn word_timing(py: Python<'_>, word: &Bound<'_, PyAny>) -> Result<WordTiming, RivaCallError> {
    let text = word_text(py, word)?;
    let start_time_ms = timing(py, word, START_TIME)?;
    let end_time_ms = timing(py, word, END_TIME)?;
    Ok(WordTiming {
        word: text,
        start_time_ms,
        end_time_ms,
    })
}

fn word_text(py: Python<'_>, word: &Bound<'_, PyAny>) -> Result<String, RivaCallError> {
    match skip_missing(py, word.getattr(WORD))? {
        Some(value) => text_value(&value),
        None => Ok(String::new()),
    }
}

// int(getattr(word, name, 0) or 0): a missing or falsey timing is zero, and int() truncates
fn timing(py: Python<'_>, word: &Bound<'_, PyAny>, name: &str) -> Result<i64, RivaCallError> {
    match present(py, word, name)? {
        Some(value) => integer_value(py, &value),
        None => Ok(0),
    }
}

fn integer_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> Result<i64, RivaCallError> {
    let builtin = py
        .import("builtins")
        .and_then(|module| module.getattr("int"))
        .map_err(|error| provider_failure(py, &error))?;
    let converted = provider_call(py, builtin.call1((value,)))?;
    match converted.extract::<i64>() {
        Ok(number) => Ok(number),
        Err(_error) => Err(RivaCallError::Malformed {
            message: String::from("a word timing is not an integer"),
        }),
    }
}
