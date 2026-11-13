use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_openai::api::audio_file_input::decoder::{
    AudioDecodeLimits, AudioDecoder, NativeAudioDecoder,
};
use betterlite_openai::api::audio_file_input::duration::calculate_request_duration;
use betterlite_openai::api::audio_file_input::hashing::get_audio_file_content_hash;
use betterlite_openai::api::audio_file_input::language::normalize_language_to_bcp47;
use betterlite_openai::api::audio_file_input::{
    HEALTH_AUDIO_FILE_NAME, HEALTH_CHECK_AUDIO, get_audio_file_name, open_health_check_audio,
    process_audio_file,
};
use serde::Deserialize;
use serde_json::{Value, json};

mod fixtures;

use self::fixtures::{File, FixtureFiles};

const HEALTH_DIRECTORY: &str = "/health";

#[derive(Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Process(FileCall),
    Hash(FileCall),
    Name(FileCall),
    Language(LanguageCall),
    Duration(FileCall),
    Health,
}

#[derive(Deserialize)]
struct FileCall {
    file: File,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct LanguageCall {
    language: String,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input
            .calls
            .iter()
            .map(outcome_of)
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(outcomes).map_err(failure)
    }
}

fn outcome_of(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    match call {
        Call::Process(file_call) => with_file(file_call, process_outcome),
        Call::Hash(file_call) => with_file(file_call, |file, files| {
            let hash = get_audio_file_content_hash(&mut file.as_file(), files);
            json!({ "ok": true, "value": { "hash": hash, "position": file.stream_position() } })
        }),
        Call::Name(file_call) => with_file(
            file_call,
            |file, _files| json!({ "ok": true, "value": get_audio_file_name(&file.as_file()) }),
        ),
        Call::Duration(file_call) => with_file(file_call, duration_outcome),
        Call::Language(language_call) => Ok(json!({
            "ok": true,
            "value": normalize_language_to_bcp47(&language_call.language),
        })),
        Call::Health => health_outcome(),
    }
}

fn with_file<Outcome>(
    call: &FileCall,
    outcome: Outcome,
) -> Result<Value, Box<crate::helpers::HelperError>>
where
    Outcome: FnOnce(&mut File, &FixtureFiles) -> Value,
{
    let files = fixture_files(&call.files)?;
    let mut file = call.file.clone();
    Ok(outcome(&mut file, &files))
}

fn process_outcome(file: &mut File, files: &FixtureFiles) -> Value {
    let Ok(processed) = process_audio_file(file.as_file(), files) else {
        return json!({ "ok": false });
    };
    json!({
        "ok": true,
        "value": {
            "filename": processed.filename,
            "content_type": processed.content_type,
            "content": BASE64.encode(processed.file_content),
        },
    })
}

fn duration_outcome(file: &mut File, files: &FixtureFiles) -> Value {
    let decoder = NativeAudioDecoder {
        limits: AudioDecodeLimits::default(),
    };
    let available = AudioDecoder::Available(&decoder);
    let seconds = calculate_request_duration(&mut file.as_file(), files, available);
    let duration = seconds.and_then(|decimal| decimal.to_string().parse::<f64>().ok());
    json!({ "ok": true, "value": { "duration": duration, "position": file.stream_position() } })
}

fn health_outcome() -> Result<Value, Box<crate::helpers::HelperError>> {
    let resource = Path::new(HEALTH_DIRECTORY).join(HEALTH_AUDIO_FILE_NAME);
    let mut entries = BTreeMap::new();
    entries.insert(resource, HEALTH_CHECK_AUDIO.to_vec());
    let files = FixtureFiles(entries);
    let mut handle =
        open_health_check_audio(&files, Path::new(HEALTH_DIRECTORY)).map_err(failure)?;
    let mut content = Vec::new();
    handle.read_to_end(&mut content).map_err(failure)?;
    Ok(json!({ "ok": true, "value": { "content": BASE64.encode(content) } }))
}

fn fixture_files(
    entries: &BTreeMap<String, String>,
) -> Result<FixtureFiles, Box<crate::helpers::HelperError>> {
    let mut files = BTreeMap::new();
    for (path, content) in entries {
        files.insert(
            PathBuf::from(path),
            BASE64.decode(content).map_err(failure)?,
        );
    }
    Ok(FixtureFiles(files))
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "AudioFileInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
