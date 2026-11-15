#![cfg(test)]

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::call_helpers::headers::ProviderHeaderProjection;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, AuthorizedAudioFiles};
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use super::fixtures::{
    API_BASE, API_KEY, API_VERSION, BOUNDARY, Cache, INPUT, Inputs, Recorder, audio_file,
    azure_call, finish, header, params, part, part_names, response, start, text,
};
use crate::endpoints::projection::{ClientKind, SuppliedClient};
use crate::transcription::call::AzureTranscriptionError;
use crate::transcription::native::{
    NativeError, SUPPORTED_OPENAI_PARAMS, complete_url, decode_native_transcription,
    get_error_class, map_openai_params, resolve_stt_base_url, transform_request,
    validate_environment,
};

#[rstest]
#[case::openai_transcription_route(openai_transcription_route)]
#[case::cognitive_stt_base(cognitive_stt_base)]
#[case::regional_stt_host(regional_stt_host)]
#[case::native_json_text_extraction(native_json_text_extraction)]
#[case::verbose_transcript(verbose_transcript)]
#[case::file_language_prompt(file_language_prompt)]
#[case::native_transcription_headers(native_transcription_headers)]
#[case::native_transcription_url_errors(native_transcription_url_errors)]
#[case::azure_transcription_async_version(azure_transcription_async_version)]
fn transcription_cases(#[case] case: fn()) {
    case();
}

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no authorized files",
        ))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no authorized files",
        ))
    }
}

fn native_headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// audio_transcriptions on the sync client: the deployment URL, the api-key header, the multipart order and the two log calls
fn openai_transcription_route() {
    let file = audio_file();
    let mut call = azure_call(&file);
    call.optional_params = params(&[
        ("language", text("en")),
        ("prompt", text("hello")),
        ("response_format", text("json")),
    ]);
    let inputs = Inputs::default();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    let (request, pending) =
        start(&inputs, &mut cache, &call, &mut logger).expect("the call starts");
    assert_openai_request(&request);
    let reply = finish(
        pending,
        response(200, "application/json", r#"{"text":"hi"}"#),
        &mut logger,
    )
    .expect("the reply decodes");
    assert_openai_reply(&reply);
    assert_openai_logging(&logger, &reply);
}

fn assert_openai_request(request: &OutboundRequest) {
    assert_eq!(request.method, http::Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/deployments/whisper/audio/transcriptions?api-version=2024-06-01"
    );
    assert_eq!(header(request, "api-key"), Some(API_KEY));
    assert_eq!(header(request, "accept"), Some("application/json"));
    assert_eq!(header(request, "authorization"), None);
    assert_eq!(
        part_names(request),
        ["model", "language", "prompt", "response_format", "file"]
    );
    assert_eq!(part(request, "model"), b"whisper");
    assert_eq!(part(request, "file"), b"abc");
}

fn assert_openai_reply(reply: &TranscriptionReply) {
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    assert_eq!(reply.model.as_deref(), Some("whisper"));
    assert_eq!(reply.custom_llm_provider, "azure");
    assert_eq!(reply.duration, StoredDuration::Absent);
    assert_eq!(reply.provider_headers, ProviderHeaderProjection::default());
    assert_eq!(
        reply.original_response,
        serde_json::from_str::<Json>(
            r#"{"text":"hi","languages":null,"logprobs":null,"usage":null}"#
        )
        .expect("the expected dump is JSON")
    );
}

fn assert_openai_logging(logger: &Recorder, reply: &TranscriptionReply) {
    let pre = logger.pre.as_ref().expect("pre_call ran");
    assert_eq!(
        pre.input,
        format!("audio_file_{}", Uuid::from_u128(BOUNDARY))
    );
    assert_eq!(pre.authorization, format!("Bearer {API_KEY}"));
    assert_eq!(pre.api_base, "https://example.openai.azure.com/openai/");
    assert_eq!(pre.data.get("model"), Some(&text("whisper")));
    assert_eq!(pre.data.get("language"), Some(&text("en")));
    let post = logger.posts.first().expect("post_call ran");
    assert_eq!(post.input, INPUT);
    assert_eq!(post.authorization, None);
    assert_eq!(post.api_base, None);
    assert_eq!(post.original_response, reply.original_response);
    assert_eq!(
        post.data.as_ref().and_then(|data| data.get("model")),
        Some(&text("whisper"))
    );
}

// _resolve_stt_base_url: a Cognitive host becomes the regional STT host, an STT host keeps its scheme and host, any other base keeps its trimmed text
fn cognitive_stt_base() {
    for (base, expected) in [
        (
            "https://api.cognitive.microsoft.com",
            "https://stt.speech.microsoft.com",
        ),
        (
            "https://eastus.api.cognitive.microsoft.com/",
            "https://eastus.stt.speech.microsoft.com",
        ),
        (
            "https://westeurope.api.cognitive.microsoft.com///",
            "https://westeurope.stt.speech.microsoft.com",
        ),
        (
            "https://eastus.stt.speech.microsoft.com",
            "https://eastus.stt.speech.microsoft.com",
        ),
        (
            "https://eastus.stt.speech.microsoft.com:8443/path?query=1",
            "https://eastus.stt.speech.microsoft.com",
        ),
        ("https://speech.test/custom/", "https://speech.test/custom"),
    ] {
        assert_eq!(resolve_stt_base_url(base).expect(base), expected, "{base}");
    }
    let refusal = resolve_stt_base_url("https://example.openai.azure.com").expect_err("refused");
    assert_eq!(refusal.status(), Some(400));
    assert_eq!(
        refusal.to_string(),
        "Azure AI Speech transcription requires a Cognitive Services or STT Speech endpoint, not an Azure OpenAI endpoint."
    );
    let resolved = complete_url(
        Some("https://eastus.api.cognitive.microsoft.com"),
        None,
        &params(&[]),
    )
    .expect("the cognitive base resolves");
    assert!(resolved.starts_with("https://eastus.stt.speech.microsoft.com/"));
}

// get_complete_url and map_openai_params: the fixed path, the language default, the detailed/simple projection and the two mapped names
fn regional_stt_host() {
    let base_text = "https://eastus.stt.speech.microsoft.com";
    assert_stt_urls(base_text);
    assert_other_formats_map_to_simple(Some(base_text));
    assert_supported_mapping();
}

fn assert_stt_urls(base_text: &str) {
    let base = Some(base_text);
    let endpoint = "/speech/recognition/conversation/cognitiveservices/v1";
    assert_eq!(
        complete_url(base, None, &params(&[])).expect("the default url"),
        format!("{base_text}{endpoint}?language=en-US&format=simple")
    );
    assert_eq!(
        complete_url(base, None, &params(&[("language", text("de DE"))]))
            .expect("the language url"),
        format!("{base_text}{endpoint}?language=de+DE&format=simple")
    );
    assert_eq!(
        complete_url(base, None, &params(&[("language", text(""))])).expect("the empty language"),
        format!("{base_text}{endpoint}?language=&format=simple")
    );
    assert_eq!(
        complete_url(
            base,
            None,
            &params(&[("response_format", text("verbose_json"))]),
        )
        .expect("the verbose url"),
        format!("{base_text}{endpoint}?language=en-US&format=detailed")
    );
}

fn assert_other_formats_map_to_simple(base: Option<&str>) {
    for format in ["json", "text", "srt", "vtt", "diarized_json"] {
        let mapped = params(&[("response_format", text(format))]);
        assert!(
            complete_url(base, None, &mapped)
                .expect("the simple url")
                .ends_with("&format=simple"),
            "{format}"
        );
    }
}

fn assert_supported_mapping() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS.to_vec(),
        vec!["language", "response_format"]
    );
    let non_default = params(&[
        ("prompt", text("ignored")),
        ("language", text("fr")),
        ("response_format", text("verbose_json")),
        ("temperature", Json::Number(Number::from(2))),
    ]);
    let existing = params(&[
        ("language", text("en")),
        ("temperature", Json::Number(Number::from(1))),
    ]);
    assert_eq!(
        map_openai_params(&non_default, existing),
        params(&[
            ("language", text("fr")),
            ("temperature", Json::Number(Number::from(1))),
            ("response_format", text("verbose_json")),
        ])
    );
}

// transform_audio_transcription_response: missing/None/Success pass, any other status fails with its raw status, DisplayText wins even empty, NBest fills in, whole body kept
fn native_json_text_extraction() {
    assert_passing_statuses();
    assert_display_text_extraction();
    assert_recognition_failures();
}

fn assert_passing_statuses() {
    for body in [
        r#"{"DisplayText":"hello"}"#,
        r#"{"RecognitionStatus":null,"DisplayText":"hello"}"#,
        r#"{"RecognitionStatus":"Success","DisplayText":"hello"}"#,
    ] {
        let reply = decode_native_transcription(200, &native_headers(&[]), body.as_bytes())
            .expect("the status passes");
        assert_eq!(reply.response.text.as_deref(), Some("hello"));
        assert_eq!(
            reply.hidden_params,
            serde_json::from_str::<Json>(body).expect("the body is JSON")
        );
    }
}

fn assert_display_text_extraction() {
    for (body, expected) in [
        (r#"{"DisplayText":""}"#, ""),
        (r#"{"DisplayText":"","NBest":[{"Display":"best"}]}"#, ""),
        (r#"{"NBest":[{"Display":"best"}]}"#, "best"),
        (
            r#"{"NBest":[{"Display":"","Lexical":"lexical"}]}"#,
            "lexical",
        ),
        (r#"{"NBest":[{"Lexical":"lexical"}]}"#, "lexical"),
        (r#"{"NBest":[]}"#, ""),
        (r#"{}"#, ""),
    ] {
        let reply = decode_native_transcription(200, &native_headers(&[]), body.as_bytes())
            .expect("the body decodes");
        assert_eq!(reply.response.text.as_deref(), Some(expected), "{body}");
    }
}

fn assert_recognition_failures() {
    for (status, headers, value) in [
        (
            200_u16,
            native_headers(&[("x-request-id", "req-1")]),
            "InitialSilenceTimeout",
        ),
        (503, native_headers(&[("retry-after", "1")]), "Error"),
    ] {
        let body = format!(r#"{{"RecognitionStatus":"{value}","DisplayText":"ignored"}}"#);
        let error = decode_native_transcription(status, &headers, body.as_bytes())
            .expect_err("the status fails");
        assert_eq!(error.status(), Some(status));
        assert_eq!(
            error.to_string(),
            format!("Azure AI Speech transcription failed with RecognitionStatus={value}.")
        );
        assert_eq!(error.headers(), Some(&headers));
    }
    let error = get_error_class(
        String::from("Azure AI Speech transcription failed."),
        401,
        native_headers(&[("x-request-id", "req-1")]),
    );
    assert_eq!(error.status(), Some(401));
    assert_eq!(error.to_string(), "Azure AI Speech transcription failed.");
}

// verbose_transcript: the sync branch projects the SDK dump, the async branch keeps its headers and adds the SRT/VTT duration, and only the async branch logs a failure
fn verbose_transcript() {
    let inputs = Inputs::default();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    sync_verbose_phase(&inputs, &mut cache, &mut logger);
    async_textual_phase(&inputs, &mut cache, &mut logger);
    async_failure_phase(&inputs, &mut cache, &mut logger);
    sync_failure_phase(&inputs, &mut cache, &mut logger);
}

fn sync_verbose_phase(inputs: &Inputs, cache: &mut Cache, logger: &mut Recorder) {
    let file = audio_file();
    let mut call = azure_call(&file);
    call.optional_params = params(&[("response_format", text("verbose_json"))]);
    let (_, pending) = start(inputs, cache, &call, logger).expect("the sync call starts");
    let reply = finish(
        pending,
        response(
            200,
            "application/json",
            r#"{"text":"hi","language":"en","duration":1.5,"segments":[],"words":[],"usage":{"type":"duration","seconds":2}}"#,
        ),
        logger,
    )
    .expect("the verbose reply decodes");
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    assert_eq!(reply.duration, StoredDuration::Absent);
    assert_eq!(
        reply.original_response,
        serde_json::from_str::<Json>(
            r#"{"duration":1.5,"language":"en","text":"hi","segments":[],"usage":{"seconds":2,"type":"duration"},"words":[]}"#
        )
        .expect("the expected dump is JSON")
    );
    assert_eq!(logger.posts.len(), 1);
}

fn async_textual_phase(inputs: &Inputs, cache: &mut Cache, logger: &mut Recorder) {
    let body = "WEBVTT\n\n00:00:00.000 --> 00:00:01.500\nhi\n";
    let file = audio_file();
    let mut call = azure_call(&file);
    call.atranscription = true;
    let (_, pending) = start(inputs, cache, &call, logger).expect("the async call starts");
    let reply = finish(pending, response(200, "text/plain", body), logger)
        .expect("the textual reply decodes");
    assert_eq!(reply.response.text.as_deref(), Some(body));
    let Json::Object(fields) = &reply.original_response else {
        panic!("the textual reply keeps an object");
    };
    assert_eq!(fields.get("text"), Some(&text(body)));
    assert_eq!(
        fields.get("_audio_transcription_duration"),
        Some(&Json::Number(
            Number::from_f64(1.5).expect("1.5 is a number")
        ))
    );
    assert_eq!(
        reply.provider_headers.headers.get("content-type"),
        Some(&String::from("text/plain"))
    );
    let post = logger.posts.last().expect("the async post_call ran");
    assert_eq!(
        post.authorization,
        Some(format!("Bearer {API_KEY}")).as_deref()
    );
    assert_eq!(
        post.api_base.as_deref(),
        Some("https://example.openai.azure.com/openai/")
    );
}

fn async_failure_phase(inputs: &Inputs, cache: &mut Cache, logger: &mut Recorder) {
    let file = audio_file();
    let mut call = azure_call(&file);
    call.atranscription = true;
    let before = logger.posts.len();
    let (_, pending) = start(inputs, cache, &call, logger).expect("the failing call starts");
    let error = finish(
        pending,
        response(500, "application/json", r#"{"error":{"message":"boom"}}"#),
        logger,
    )
    .expect_err("the failed reply is an error");
    assert_eq!(error.status(), Some(500));
    let post = logger.posts.last().expect("the failure is logged");
    assert_eq!(logger.posts.len(), before + 1);
    assert_eq!(post.input, INPUT);
    assert_eq!(post.original_response, text("boom"));
    assert_eq!(post.data, None);
    assert_eq!(post.authorization, None);
}

fn sync_failure_phase(inputs: &Inputs, cache: &mut Cache, logger: &mut Recorder) {
    let file = audio_file();
    let call = azure_call(&file);
    let before = logger.posts.len();
    let (_, pending) = start(inputs, cache, &call, logger).expect("the sync call starts");
    let error = finish(pending, response(500, "application/json", "{}"), logger)
        .expect_err("the sync failure is an error");
    assert_eq!(error.status(), Some(500));
    assert_eq!(logger.posts.len(), before);
}

// file_language_prompt: the native request hands process_audio_file bytes and content type with no files mapping, and {model, file, **options} lets options override
fn file_language_prompt() {
    native_request_transform();
    override_request_and_reply();
}

fn native_request_transform() {
    let request =
        transform_request(AudioFile::Bytes(b"abc"), &NoFiles).expect("the bytes transform");
    assert_eq!(request.data, AudioTranscriptionData::Bytes(b"abc".to_vec()));
    assert_eq!(request.files, None);
    assert_eq!(request.content_type.as_deref(), Some("audio/wav"));
    let path = PathBuf::from("missing.wav");
    assert!(matches!(
        transform_request(AudioFile::Path(&path), &NoFiles),
        Err(NativeError::AudioFile(_))
    ));
}

fn override_request_and_reply() {
    let file = audio_file();
    let mut call = azure_call(&file);
    call.optional_params = params(&[
        ("model", text("whisper-override")),
        ("language", text("fr")),
        ("prompt", text("hello")),
    ]);
    call.azure_ad_token = Some(SecretString::from("unforwarded-token"));
    call.max_retries = Some(7);
    let inputs = Inputs::default();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    let (request, pending) =
        start(&inputs, &mut cache, &call, &mut logger).expect("the call starts");
    assert_eq!(
        part_names(&request),
        ["model", "language", "prompt", "file"]
    );
    assert_eq!(part(&request, "model"), b"whisper-override");
    assert_eq!(part(&request, "language"), b"fr");
    assert_eq!(part(&request, "prompt"), b"hello");
    assert_eq!(header(&request, "api-key"), Some(API_KEY));
    assert_eq!(header(&request, "authorization"), None);
    let pre = logger.pre.as_ref().expect("pre_call ran");
    assert_eq!(pre.data.get("model"), Some(&text("whisper-override")));
    let reply = finish(
        pending,
        response(200, "application/json", r#"{"text":"hi"}"#),
        &mut logger,
    )
    .expect("the reply decodes");
    assert_eq!(reply.model.as_deref(), Some("whisper"));
}

// validate_environment: the key comes from the argument or AZURE_SPEECH_API_KEY, an absent key is a 401, and a caller Content-Type survives
fn native_transcription_headers() {
    explicit_key_headers();
    environment_and_missing_keys();
}

fn explicit_key_headers() {
    let caller = native_headers(&[
        ("Ocp-Apim-Subscription-Key", "stale"),
        ("Content-Type", "audio/wav; codecs=1"),
        ("X-Trace", "1"),
    ]);
    let explicit = SecretString::from("explicit-key");
    let validated = validate_environment(&caller, Some(&explicit), None).expect("the explicit key");
    assert_eq!(
        validated,
        native_headers(&[
            ("Ocp-Apim-Subscription-Key", "explicit-key"),
            ("Content-Type", "audio/wav; codecs=1"),
            ("X-Trace", "1"),
            ("Accept", "application/json"),
        ])
    );
}

fn environment_and_missing_keys() {
    let explicit = SecretString::from("explicit-key");
    let environment = SecretString::from("environment-key");
    let defaulted =
        validate_environment(&native_headers(&[]), None, Some(&environment)).expect("the env key");
    assert_eq!(
        defaulted,
        native_headers(&[
            ("Ocp-Apim-Subscription-Key", "environment-key"),
            ("Content-Type", "audio/wav"),
            ("Accept", "application/json"),
        ])
    );
    let preferred = validate_environment(&native_headers(&[]), Some(&explicit), Some(&environment))
        .expect("both");
    assert_eq!(
        preferred
            .get("Ocp-Apim-Subscription-Key")
            .map(String::as_str),
        Some("explicit-key")
    );
    let missing = validate_environment(&native_headers(&[]), None, None).expect_err("no key");
    assert_eq!(missing.status(), Some(401));
    assert_eq!(
        missing.to_string(),
        "api_key is required for Azure AI Speech transcription."
    );
}

// get_complete_url: an absent base is a 400 naming both endpoint families, and an Azure OpenAI host is refused with its own 400
fn native_transcription_url_errors() {
    assert_base_errors();
    assert_host_rules();
}

fn assert_base_errors() {
    let missing = complete_url(None, None, &params(&[])).expect_err("no base");
    assert_eq!(missing.status(), Some(400));
    assert_eq!(
        missing.to_string(),
        "api_base is required for Azure AI Speech transcription. Use a Cognitive Services endpoint like https://{region}.api.cognitive.microsoft.com or an STT endpoint like https://{region}.stt.speech.microsoft.com."
    );
    assert_eq!(
        complete_url(
            None,
            Some("https://eastus.api.cognitive.microsoft.com/"),
            &params(&[])
        )
        .expect("the environment base"),
        "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
    assert_eq!(
        complete_url(
            Some("https://speech.test"),
            Some("https://ignored.test"),
            &params(&[])
        )
        .expect("the argument base wins"),
        "https://speech.test/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
}

fn assert_host_rules() {
    let refusal = complete_url(
        Some("https://example.openai.azure.com/"),
        None,
        &params(&[]),
    )
    .expect_err("the Azure OpenAI host is refused");
    assert_eq!(refusal.status(), Some(400));
    assert_eq!(
        refusal.to_string(),
        "Azure AI Speech transcription requires a Cognitive Services or STT Speech endpoint, not an Azure OpenAI endpoint."
    );
    // a host that merely ends in the letters is a custom base, not an Azure OpenAI endpoint
    assert_eq!(
        complete_url(
            Some("https://example.openai.azure.com.evil"),
            None,
            &params(&[])
        )
        .expect("the custom base"),
        "https://example.openai.azure.com.evil/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
    let wrong = params(&[("language", Json::Bool(true))]);
    let field =
        complete_url(Some("https://speech.test"), None, &wrong).expect_err("the type is wrong");
    assert_eq!(
        field.to_string(),
        "the transcription field `language` is not a string"
    );
}

// The atranscription branch drops api_version before the async helper resolves its own, and each branch accepts only its exact client kind
fn azure_transcription_async_version() {
    sync_and_async_versions();
    version_one_is_rejected();
    supplied_openai_is_rejected();
}

fn sync_and_async_versions() {
    let file = audio_file();
    let inputs = Inputs::default();
    let mut logger = Recorder::default();
    let call = azure_call(&file);
    let mut sync_cache = Cache::default();
    let (request, _) =
        start(&inputs, &mut sync_cache, &call, &mut logger).expect("the sync call starts");
    assert!(request.url.as_str().ends_with("?api-version=2024-06-01"));
    let mut async_call = azure_call(&file);
    async_call.atranscription = true;
    let mut async_cache = Cache::default();
    let (request, _) =
        start(&inputs, &mut async_cache, &async_call, &mut logger).expect("the async call starts");
    assert!(
        request
            .url
            .as_str()
            .ends_with("?api-version=2025-02-01-preview"),
        "{}",
        request.url
    );
}

fn version_one_is_rejected() {
    let file = audio_file();
    let inputs = Inputs::default();
    let mut logger = Recorder::default();
    let mut version_one = azure_call(&file);
    version_one.api_version = Some(String::from("v1"));
    let mut version_one_cache = Cache::default();
    let sync = start(&inputs, &mut version_one_cache, &version_one, &mut logger)
        .expect_err("sync rejects v1");
    assert_eq!(sync.status(), Some(500));
    assert_eq!(
        sync.to_string(),
        "azure_client is not an instance of AzureOpenAI"
    );
    let mut version_one_async = azure_call(&file);
    version_one_async.api_version = Some(String::from("v1"));
    version_one_async.atranscription = true;
    let mut version_one_async_cache = Cache::default();
    let (request, _) = start(
        &inputs,
        &mut version_one_async_cache,
        &version_one_async,
        &mut logger,
    )
    .expect("the async branch drops the v1 version");
    assert!(
        request
            .url
            .as_str()
            .ends_with("?api-version=2025-02-01-preview"),
        "{}",
        request.url
    );
}

fn supplied_openai_is_rejected() {
    let file = audio_file();
    let inputs = Inputs::default();
    let mut logger = Recorder::default();
    let mut supplied_openai = azure_call(&file);
    supplied_openai.supplied = Some(SuppliedClient {
        kind: ClientKind::OpenAi,
        custom_query: None,
    });
    supplied_openai.atranscription = true;
    let mut supplied_cache = Cache::default();
    let async_error = start(&inputs, &mut supplied_cache, &supplied_openai, &mut logger)
        .expect_err("async rejects a supplied OpenAI client");
    assert_eq!(async_error.status(), Some(500));
    assert_eq!(
        async_error.to_string(),
        "async_azure_client is not an instance of AsyncAzureOpenAI"
    );
}

// the two stand-alone invariants the task names
#[test]
fn azure_transcription_credential_identity_isolation() {
    let file = audio_file();
    let inputs = Inputs::default();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    let first = azure_call(&file);
    let (first_request, _) =
        start(&inputs, &mut cache, &first, &mut logger).expect("the first call");
    let mut second = azure_call(&file);
    second.api_key = Some(SecretString::from("sk-other"));
    let (second_request, _) =
        start(&inputs, &mut cache, &second, &mut logger).expect("the second call");
    assert_eq!(header(&first_request, "api-key"), Some(API_KEY));
    assert_eq!(header(&second_request, "api-key"), Some("sk-other"));
    assert_eq!(cache.len(), 2);
    let mut same = azure_call(&file);
    same.api_key = Some(SecretString::from("sk-other"));
    let (same_request, _) =
        start(&inputs, &mut cache, &same, &mut logger).expect("the cached call");
    assert_eq!(header(&same_request, "api-key"), Some("sk-other"));
    assert_eq!(cache.len(), 2);
    let mut token = azure_call(&file);
    token.params.azure_ad_token = Some(SecretString::from("ad-token"));
    let (token_request, _) =
        start(&inputs, &mut cache, &token, &mut logger).expect("the token call");
    assert_eq!(
        header(&token_request, "authorization"),
        Some("Bearer ad-token")
    );
    assert_eq!(header(&token_request, "api-key"), None);
    assert_eq!(cache.len(), 3);
}

#[test]
fn azure_transcription_secret_safe_failure() {
    let file = audio_file();
    let secret = "sk-super-secret\nvalue";
    let mut call = azure_call(&file);
    call.api_key = Some(SecretString::from(secret));
    let inputs = Inputs::default();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    let error =
        start(&inputs, &mut cache, &call, &mut logger).expect_err("the header refuses the key");
    assert!(matches!(error, AzureTranscriptionError::Header { .. }));
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert!(!debug.contains("sk-super-secret"), "{debug}");
    assert!(!display.contains("sk-super-secret"), "{display}");
    let missing = validate_environment(&native_headers(&[]), None, None).expect_err("no key");
    assert!(!format!("{missing:?}").contains("sk-super-secret"));
}
