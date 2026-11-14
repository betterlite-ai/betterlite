use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;

use betterlite_config::types::LiteLlmParams;
use betterlite_core::api::elevenlabs::transcription::auth::{
    TranscriptionKeys, validate_environment,
};
use betterlite_core::api::elevenlabs::transcription::endpoint::complete_url;
use betterlite_core::api::elevenlabs::transcription::mapping::{
    SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_core::api::elevenlabs::transcription::request::transform_request;
use betterlite_core::api::elevenlabs::transcription::response::{
    MAX_REPLY_BYTES, decode_eleven_transcript,
};
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent, process_audio_file,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    MapParams {
        non_default: Object,
        optional_params: Object,
    },
    Request {
        model: String,
        filename: Option<String>,
        content: String,
        optional_params: Object,
    },
    Url {
        api_base: Option<String>,
        environment: BTreeMap<String, String>,
    },
    Headers {
        headers: IndexMap<String, String>,
        api_key: Option<String>,
        environment: BTreeMap<String, String>,
    },
    Transcript {
        body: String,
    },
}

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let answers = input.calls.iter().map(answer).collect();
        Ok(json!({ "calls": Value::Array(answers) }))
    }
}

fn answer(call: &Call) -> Value {
    match call {
        Call::MapParams {
            non_default,
            optional_params,
        } => map_answer(non_default, optional_params),
        Call::Request {
            model,
            filename,
            content,
            optional_params,
        } => request_answer(model, filename.as_deref(), content, optional_params),
        Call::Url {
            api_base,
            environment,
        } => url_answer(api_base.as_deref(), environment),
        Call::Headers {
            headers,
            api_key,
            environment,
        } => headers_answer(headers, api_key.as_ref(), environment),
        Call::Transcript { body } => transcript_answer(body),
    }
}

fn map_answer(non_default: &Object, optional_params: &Object) -> Value {
    let mut mapped = optional_params.clone();
    map_openai_params(non_default, &mut mapped);
    json!({
        "ok": true,
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
    })
}

fn request_answer(
    model: &str,
    filename: Option<&str>,
    content: &str,
    optional_params: &Object,
) -> Value {
    let bytes = content.as_bytes();
    let file = filename.map_or(AudioFile::Bytes(bytes), |name| {
        AudioFile::Tuple(AudioTuple {
            filename: name,
            content: Some(TupleContent::Bytes(bytes)),
        })
    });
    let Ok(audio) = process_audio_file(file, &NoFiles) else {
        return json!({ "ok": false });
    };
    transform_request(model, &audio, optional_params).map_or_else(
        |_| json!({ "ok": false }),
        |form| {
            let fields: Vec<Value> = form
                .fields
                .iter()
                .map(|(name, value)| json!([name, value]))
                .collect();
            json!({
                "ok": true,
                "form": fields,
                "file": {
                    "name": form.file.name,
                    "filename": form.file.file_name,
                    "content_type": form.file.content_type,
                    "bytes": form.file.data,
                },
            })
        },
    )
}

fn url_answer(api_base: Option<&str>, environment: &BTreeMap<String, String>) -> Value {
    let (_, environment_base) = LiteLlmParams::elevenlabs_speech_settings(environment);
    json!({ "ok": true, "url": complete_url(api_base, &environment_base) })
}

fn headers_answer(
    headers: &IndexMap<String, String>,
    api_key: Option<&String>,
    environment: &BTreeMap<String, String>,
) -> Value {
    let (environment_key, _) = LiteLlmParams::elevenlabs_speech_settings(environment);
    let explicit = api_key.cloned().map(SecretString::from);
    let keys = TranscriptionKeys {
        explicit: explicit.as_ref(),
        environment: environment_key.as_ref(),
    };
    validate_environment(headers, &keys).map_or_else(
        |_| json!({ "ok": false }),
        |updated| json!({ "ok": true, "headers": updated }),
    )
}

fn transcript_answer(body: &str) -> Value {
    decode_eleven_transcript(body.as_bytes(), MAX_REPLY_BYTES).map_or_else(
        |_| json!({ "ok": false }),
        |found| {
            let hidden = found.hidden.clone();
            json!({ "ok": true, "response": found, "hidden": hidden })
        },
    )
}
