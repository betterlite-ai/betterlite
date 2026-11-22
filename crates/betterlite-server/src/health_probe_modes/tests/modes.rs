use betterlite_openai::api::audio_file_input::HEALTH_CHECK_AUDIO;
use secrecy::ExposeSecret as _;

use self::support::{DOCUMENT_URL, FakeDocuments, with_input};
use super::params::HealthProbeParams;

#[rstest]
#[case::chat("chat")]
#[case::completion("completion")]
#[case::embedding("embedding")]
#[case::audio_speech("audio_speech")]
#[case::audio_transcription("audio_transcription")]
#[case::image_generation("image_generation")]
#[case::image_edit("image_edit")]
#[case::video_generation("video_generation")]
#[case::rerank("rerank")]
#[case::realtime("realtime")]
#[case::batch("batch")]
#[case::responses("responses")]
#[case::ocr("ocr")]
#[case::evaluation("evaluation")]
#[tokio::test]
async fn mode_each_mode_sends_its_own_call(#[case] mode: &str) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"m","messages":[]}"#);
    run_mode(
        mode,
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    assert_eq!(backend.take_mode(), mode);
}

#[test]
fn mode_unknown_name_has_no_plan() {
    let args = HealthProbeArguments::default();
    let plan = plan_health_probe("vision", "m", "openai", HealthProbeParams::default(), args);
    assert!(plan.is_none());
}

#[tokio::test]
async fn mode_chat_keeps_messages_and_every_parameter() {
    let backend = FakeBackend::new();
    let entries =
        r#"{"model":"gpt-5","messages":[{"role":"user","content":"hi"}],"temperature":0.2}"#;
    let params = probe_params_from_json(entries);
    run_mode(
        "chat",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_chat();
    assert!(call.parameters.contains_key("messages"));
    let temperature = serde_json::to_string(call.parameters.get("temperature").unwrap()).unwrap();
    assert_eq!(temperature, "0.2");
}

#[tokio::test]
async fn mode_cache_marker_reaches_the_probe_call() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"gpt-5","cache":{"no-cache":true}}"#);
    run_mode(
        "completion",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_completion();
    let cache = serde_json::to_string(call.parameters.get("cache").unwrap()).unwrap();
    assert_eq!(cache, r#"{"no-cache":true}"#);
}

#[rstest]
#[case::missing(None, "test")]
#[case::empty(Some(""), "test")]
#[case::given(Some("say hi"), "say hi")]
#[tokio::test]
async fn mode_completion_sends_the_given_prompt_or_the_test_prompt(
    #[case] prompt: Option<&str>,
    #[case] expected: &str,
) {
    let backend = FakeBackend::new();
    let entries = r#"{"model":"gpt-5","messages":[{"role":"user","content":"hi"}]}"#;
    let params = probe_params_from_json(entries);
    run_mode(
        "completion",
        "openai",
        params,
        with_prompt(prompt),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_completion();
    assert!(!call.parameters.contains_key("messages"));
    assert_eq!(
        call.parameters.get("prompt"),
        Some(&Json::Text(expected.to_owned()))
    );
}

#[rstest]
#[case::missing(None, r#"["test"]"#)]
#[case::empty(Some(vec![]), r#"["test"]"#)]
#[case::given(Some(vec!["alpha", "beta"]), r#"["alpha","beta"]"#)]
#[tokio::test]
async fn mode_embedding_sends_the_given_input_or_the_test_input(
    #[case] input: Option<Vec<&str>>,
    #[case] expected: &str,
) {
    let backend = FakeBackend::new();
    let entries = r#"{"model":"text-embedding-3-small","messages":[]}"#;
    let params = probe_params_from_json(entries);
    run_mode("embedding", "openai", params, with_input(input), &backend)
        .await
        .unwrap();
    let call = backend.take_embedding();
    assert!(!call.parameters.contains_key("messages"));
    let sent = serde_json::to_string(call.parameters.get("input").unwrap()).unwrap();
    assert_eq!(sent, expected);
}

#[rstest]
#[case::missing(r#"{"model":"tts"}"#, r#""alloy""#)]
#[case::given(r#"{"model":"tts","voice":"nova"}"#, r#""nova""#)]
#[case::null(r#"{"model":"tts","voice":null}"#, "null")]
#[tokio::test]
async fn mode_audio_speech_adds_alloy_only_without_a_voice_key(
    #[case] entries: &str,
    #[case] expected: &str,
) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(entries);
    run_mode(
        "audio_speech",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_audio_speech();
    let voice = serde_json::to_string(call.parameters.get("voice").unwrap()).unwrap();
    assert_eq!(voice, expected);
    assert_eq!(
        call.parameters.get("input"),
        Some(&Json::Text("test".to_owned()))
    );
}

#[tokio::test]
async fn mode_audio_transcription_sends_the_health_wav() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"whisper-1","messages":[],"language":"en"}"#);
    let reply = run_mode(
        "audio_transcription",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    assert_eq!(reply, HealthProbeReply::default());
    let call = backend.take_audio_transcription();
    assert_eq!(call.file, HEALTH_CHECK_AUDIO);
    assert!(!call.parameters.contains_key("messages"));
    assert_eq!(
        call.parameters.get("language"),
        Some(&Json::Text("en".to_owned()))
    );
}

#[rstest]
#[case::missing(None, None)]
#[case::empty(Some(""), Some(""))]
#[case::given(Some("a blue circle"), Some("a blue circle"))]
#[tokio::test]
async fn mode_image_generation_keeps_the_prompt_as_given(
    #[case] prompt: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"dall-e-3","messages":[]}"#);
    run_mode(
        "image_generation",
        "openai",
        params,
        with_prompt(prompt),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_image_generation();
    assert!(!call.parameters.contains_key("messages"));
    let prompt_value = expected.map_or(Json::Null, |text| Json::Text(text.to_owned()));
    assert_eq!(call.parameters.get("prompt"), Some(&prompt_value));
}

#[rstest]
#[case::missing(None, "test video generation")]
#[case::given(Some("a cat"), "a cat")]
#[tokio::test]
async fn mode_video_generation_sends_the_given_prompt_or_the_default(
    #[case] prompt: Option<&str>,
    #[case] expected: &str,
) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"veo","messages":[]}"#);
    run_mode(
        "video_generation",
        "openai",
        params,
        with_prompt(prompt),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_video_generation();
    assert!(!call.parameters.contains_key("messages"));
    assert_eq!(
        call.parameters.get("prompt"),
        Some(&Json::Text(expected.to_owned()))
    );
}

#[rstest]
#[case::missing(None, "")]
#[case::given(Some("capital of France?"), "capital of France?")]
#[tokio::test]
async fn mode_rerank_sends_the_given_query_or_an_empty_one(
    #[case] prompt: Option<&str>,
    #[case] expected: &str,
) {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"rerank-v3","messages":[]}"#);
    run_mode("rerank", "openai", params, with_prompt(prompt), &backend)
        .await
        .unwrap();
    let call = backend.take_rerank();
    assert!(!call.parameters.contains_key("messages"));
    assert_eq!(
        call.parameters.get("query"),
        Some(&Json::Text(expected.to_owned()))
    );
    let documents = serde_json::to_string(call.parameters.get("documents").unwrap()).unwrap();
    assert_eq!(documents, r#"["my sample text"]"#);
}

#[tokio::test]
async fn mode_realtime_passes_the_identity_and_the_full_params() {
    let backend = FakeBackend::new();
    let entries = concat!(
        r#"{"model":"ignored-by-the-plan","api_base":"https://api.test","#,
        r#""api_version":"2024-01-01","api_key":"sk-test","#,
        r#""messages":[{"role":"user","content":"hi"}]}"#
    );
    let params = probe_params_from_json(entries);
    run_mode(
        "realtime",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_realtime();
    assert_eq!(call.model, "resolved-model");
    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(call.api_base.as_deref(), Some("https://api.test"));
    assert_eq!(call.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(call.api_key.unwrap().expose_secret(), "sk-test");
    assert!(call.model_params.contains_key("messages"));
}

#[tokio::test]
async fn mode_responses_sends_the_test_input() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"o4-mini","messages":[]}"#);
    run_mode(
        "responses",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_responses();
    assert!(!call.parameters.contains_key("messages"));
    assert_eq!(
        call.parameters.get("input"),
        Some(&Json::Text("test".to_owned()))
    );
}

#[tokio::test]
async fn mode_ocr_sends_the_injected_document() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"text-extract","messages":[]}"#);
    run_mode(
        "ocr",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_ocr();
    assert_eq!(call.document.source(), DOCUMENT_URL);
    assert!(!call.parameters.contains_key("messages"));
}

#[tokio::test]
async fn mode_ocr_document_failure_is_lazy() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"text-extract"}"#);
    let args = HealthProbeArguments::default();
    let plan = plan_health_probe("ocr", "resolved-model", "openai", params, args).unwrap();
    let error = run_health_probe(plan, &backend, &FakeDocuments { missing: true })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        HealthProbeError::OcrDocumentUnavailable { .. }
    ));
    assert_eq!(backend.call_count(), 0);
}

#[tokio::test]
async fn mode_evaluation_validation_is_lazy() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"m","state":true}"#);
    let args = HealthProbeArguments::default();
    let plan = plan_health_probe("evaluation", "m", "openai", params, args).unwrap();
    let error = run_health_probe(plan, &backend, &FakeDocuments { missing: false })
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        HealthProbeError::EvaluationPayload { field: "state", .. }
    ));
    assert_eq!(backend.call_count(), 0);
}

#[rstest]
#[case::model(r#"{"model":7}"#, "model")]
#[case::api_base(r#"{"model":"m","api_base":7}"#, "api_base")]
#[case::api_key(r#"{"model":"m","api_key":7}"#, "api_key")]
#[tokio::test]
async fn mode_typed_parameters_with_a_wrong_type_are_rejected(
    #[case] entries: &str,
    #[case] key: &str,
) {
    let parsed = serde_json::from_str(entries).unwrap();
    let Err(error) = HealthProbeParams::from_map(parsed) else {
        panic!("a wrong parameter type is rejected");
    };
    assert!(matches!(
        error,
        HealthProbeError::ParameterType { key: reported } if reported == key
    ));
}

#[rstest]
#[case::completion("completion", "prompt")]
#[case::embedding("embedding", "input")]
#[case::audio_speech("audio_speech", "input")]
#[case::audio_transcription("audio_transcription", "file")]
#[case::image_generation("image_generation", "prompt")]
#[case::image_edit_image("image_edit", "image")]
#[case::image_edit_prompt("image_edit", "prompt")]
#[case::video_generation("video_generation", "prompt")]
#[case::rerank_query("rerank", "query")]
#[case::rerank_documents("rerank", "documents")]
#[case::responses("responses", "input")]
#[case::ocr("ocr", "document")]
#[tokio::test]
async fn mode_repeated_key_fails_without_a_call(#[case] mode: &str, #[case] key: &str) {
    let mut entries = IndexMap::new();
    entries.insert("model".to_owned(), "m");
    entries.insert(key.to_owned(), "given");
    let backend = FakeBackend::new();
    let error = run_mode(
        mode,
        "openai",
        probe_params(entries),
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        HealthProbeError::RepeatedParameter { key: reported, mode: reported_mode }
            if reported == key && reported_mode == mode
    ));
    assert_eq!(backend.call_count(), 0);
}

#[tokio::test]
async fn mode_api_key_travels_as_a_secret_not_as_a_parameter() {
    let backend = FakeBackend::new();
    let params = probe_params_from_json(r#"{"model":"gpt-5","api_key":"sk-test"}"#);
    run_mode(
        "chat",
        "openai",
        params,
        HealthProbeArguments::default(),
        &backend,
    )
    .await
    .unwrap();
    let call = backend.take_chat();
    assert!(!call.parameters.contains_key("api_key"));
    assert_eq!(call.api_key.unwrap().expose_secret(), "sk-test");
}
