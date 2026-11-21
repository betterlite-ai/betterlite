use std::future::Future;
use std::path::Path;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::error::MapperError;
use betterlite_context::non_default_params::mapper::DropPolicy;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, ProcessedAudioFile};
use betterlite_openai::transcription::config::TranscriptionFamily;
use indexmap::IndexMap;

use super::support::request_for;
use super::{
    FakeError, Harness, Recorder, TargetKind, fixed_boundary, json, params, response, runtime,
    secret, value, wav,
};
use crate::api::transcription_call::call::SuppliedClientKind;
use crate::api::transcription_call::dispatch::{TranscriptionAnswer, TranscriptionDispatch};
use crate::api::transcription_call::error::{
    AsyncTranscriptionError, TranscriptionDispatchError, TranscriptionError,
};
use crate::api::transcription_call::error_type_name;
use crate::api::transcription_call::plan::{bool_value, drop_policy};
use crate::api::transcription_call::service::Transcriptions;
use crate::api::transcription_call::target::{TranscriptionInput, TranscriptionTarget};

#[derive(Debug, thiserror::Error)]
#[error("bare dispatch failure")]
struct BareError;

impl TranscriptionDispatchError for BareError {}

// the trait's own defaults, which every operation error that overrides nothing reports through
#[test]
fn dispatch_error_defaults() {
    assert_eq!(BareError.status_code(), None);
    assert_eq!(BareError.error_str(), "bare dispatch failure");
    assert_eq!(BareError.message(), "bare dispatch failure");
}

// kwargs.get("drop_params") counts only a literal bool, as the typed kwargs contract says
#[test]
fn drop_params_reads_a_literal_bool() {
    assert_eq!(bool_value(&value("true")), Some(true));
    assert_eq!(bool_value(&value("false")), Some(false));
    assert_eq!(bool_value(&value("null")), None);
    assert_eq!(bool_value(&value("1")), None);
    assert_eq!(bool_value(&value("\"true\"")), None);
}

// the kwargs bool and litellm_settings both feed the drop policy, and only a literal bool counts
#[test]
fn drop_policy_reads_settings_and_kwargs() {
    let harness = Harness::new();
    let mut call = request_for("xai/whisper");
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: None,
            litellm_drop_params: false,
        }
    );
    call.params = params(r#"{"drop_params":true}"#);
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: Some(true),
            litellm_drop_params: false,
        }
    );
    call.params = params(r#"{"drop_params":false}"#);
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: Some(false),
            litellm_drop_params: false,
        }
    );
    call.params = params(r#"{"drop_params":"true"}"#);
    assert_eq!(
        drop_policy(&harness.settings, &call),
        DropPolicy {
            drop_params: None,
            litellm_drop_params: false,
        }
    );
    let mut configured = Harness::new();
    configured.settings.drop_params = Some(true);
    call.params = Params::new();
    assert_eq!(
        drop_policy(&configured.settings, &call),
        DropPolicy {
            drop_params: None,
            litellm_drop_params: true,
        }
    );
}

#[test]
fn optional_params_map_keeps_the_decisions_shape() {
    let processed = ProcessedAudioFile {
        file_content: Vec::new(),
        filename: String::from("a.wav"),
        content_type: String::from("audio/wav"),
    };
    let input = TranscriptionInput {
        model: "whisper-1",
        input: "a.wav",
        file: &processed,
        optional_params: params(r#"{"language":"en","temperature":0}"#),
        litellm_params: Params::new(),
        atranscription: false,
        timeout: Duration::from_secs(600),
        max_retries: 2,
        api_base: None,
        api_key: None,
        client: None,
        shared_session: None,
        boundary: fixed_boundary(),
    };
    let mapped = input.optional_params_map();
    assert_eq!(
        mapped.get("language"),
        Some(&Json::Text(String::from("en")))
    );
    assert_eq!(mapped.get("temperature"), Some(&json("0")));
}

// get_optional_params_transcription's extra_body members reach the transport beside the named
// options, and a clashing member wins, as the OpenAI SDK's own extra_body merge does
#[test]
fn extra_body_members_are_laid_over_the_openai_options() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("openai/whisper-1");
        request.language = Some(String::from("en"));
        request.params = params(r#"{"extra_body":{"language":"fr"},"custom_field":7}"#);
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the transcription succeeds");
        let recorded = harness.dispatch.last();
        assert_eq!(
            recorded.optional_params.get("language"),
            Some(&value(r#""fr""#))
        );
        assert_eq!(
            recorded.optional_params.get("custom_field"),
            Some(&value("7"))
        );
        assert!(!recorded.optional_params.contains_key("extra_body"));
    });
}

// model_call_details["client"] = str(client), with "None" when the caller passed no client
#[test]
fn logging_records_the_client_string() {
    runtime().block_on(async {
        let harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("openai/gpt-4o-transcribe");
        request.params = params(r#"{"litellm_call_id":"caller-call"}"#);
        request.user = Some(String::from("caller-user"));
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&audio))
            .await
            .expect("the transcription succeeds");
        let state = harness
            .logging
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            state.model_call_details.get("client"),
            Some(&Json::Text(String::from("None")))
        );
        // the resolved model replaces the harness INIT model, and the provider slug travels beside it
        assert_eq!(state.model, "gpt-4o-transcribe");
        assert_eq!(
            state.model_call_details.get("model"),
            Some(&Json::Text(String::from("gpt-4o-transcribe")))
        );
        assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
        assert_eq!(
            state.model_call_details.get("custom_llm_provider"),
            Some(&Json::Text(String::from("openai")))
        );
        assert_eq!(state.user.as_deref(), Some("caller-user"));
        assert!(state.optional_params.is_empty());
        // the caller's kwargs come after the six defaults, so its value replaces the default
        assert_eq!(
            state.litellm_params.get("litellm_call_id"),
            Some(&Json::Text(String::from("caller-call")))
        );
        assert_eq!(
            state.litellm_params.get("proxy_server_request"),
            Some(&Json::Null)
        );
        assert_eq!(
            state.litellm_params.get("stream_response"),
            Some(&Json::Object(IndexMap::new()))
        );
        assert_eq!(state.litellm_params.get("metadata"), Some(&Json::Null));
        assert_eq!(state.litellm_params.get("model_info"), Some(&Json::Null));
        assert_eq!(
            state.litellm_params.get("preset_cache_key"),
            Some(&Json::Null)
        );
    });
}

// the generic native handler keeps only the two HTTPHandler client classes
#[test]
fn native_http_keeps_the_http_handler_client() {
    runtime().block_on(async {
        let harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("xai/whisper");
        request.client = Some(SuppliedClientKind::HttpHandler);
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&audio))
            .await
            .expect("the native transcription succeeds");
        assert_eq!(
            harness.dispatch.last().client,
            Some(SuppliedClientKind::HttpHandler)
        );
    });
}

// a timeout Duration cannot hold is refused before any provider operation runs
#[test]
fn invalid_timeout_is_refused_before_dispatch() {
    runtime().block_on(async {
        let harness = Harness::new();
        let mut request = request_for("openai/whisper-1");
        request.timeout = Some(-1.0_f64);
        let refused = harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await;
        let Err(error) = refused else {
            panic!("a negative timeout is refused");
        };
        assert!(matches!(error, TranscriptionError::InvalidTimeout { .. }));
        assert!(harness.dispatch.targets().is_empty());
    });
}

// tags, atranscription and client are popped before the branch sees the remaining kwargs
#[test]
fn popped_kwargs_never_reach_the_provider() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("openai/whisper-1");
        request.params =
            params(r#"{"tags":["a"],"atranscription":true,"client":"x","azure_ad_token":"t"}"#);
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the transcription succeeds");
        let recorded = harness.dispatch.last();
        assert!(!recorded.litellm_params.contains_key("tags"));
        assert!(!recorded.litellm_params.contains_key("atranscription"));
        assert!(!recorded.litellm_params.contains_key("client"));
        assert_eq!(
            recorded.litellm_params.get("azure_ad_token"),
            Some(&value(r#""t""#))
        );
    });
}

// args[0] names the model ahead of the call's own field, and a missing one is the wrapper's KeyError
#[test]
fn atranscription_uses_the_first_argument_as_the_model() {
    runtime().block_on(async {
        let harness = Harness::new();
        let request = request_for("openai/gpt-4o-transcribe");
        let refusal = harness
            .service()
            .atranscription(None, &request, AudioFile::Bytes(&wav(1)), true)
            .await;
        let Err(absent_model) = refusal else {
            panic!("the model is missing");
        };
        assert!(matches!(
            absent_model,
            AsyncTranscriptionError::MissingModel
        ));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        harness
            .service()
            .atranscription(
                Some("openai/whisper-1"),
                &request,
                AudioFile::Bytes(&wav(1)),
                true,
            )
            .await
            .expect("the positional model wins");
        assert_eq!(harness.dispatch.last().model, "whisper-1");
    });
}

// an unresolvable model is mapped with "openai" as the provider fallback the wrapper uses
#[test]
fn atranscription_falls_back_to_openai_for_an_unresolvable_model() {
    runtime().block_on(async {
        let harness = Harness::new();
        let request = request_for("no-such-provider/model-x");
        let answer = harness
            .service()
            .atranscription(
                Some("no-such-provider/model-x"),
                &request,
                AudioFile::Bytes(&wav(1)),
                true,
            )
            .await;
        let Err(refusal) = answer else {
            panic!("the provider is unknown");
        };
        let AsyncTranscriptionError::Exception(exception) = refusal else {
            panic!("the resolution failure maps to an exception");
        };
        assert_eq!(
            exception.class.metadata().provider.as_deref(),
            Some("openai")
        );
        // get_llm_provider raises BadRequestError with a 400 before any provider branch runs
        assert!(matches!(exception.class, ErrorClass::BadRequest(_)));
        assert_eq!(exception.class.metadata().status_code, 400_u16);
        assert!(harness.dispatch.targets().is_empty());
    });
}

// resolution and the optional params run before any handler opens the file, as get_llm_provider does
#[test]
fn resolution_failure_precedes_the_audio_read() {
    runtime().block_on(async {
        let harness = Harness::new();
        let request = request_for("no-such-provider/model-x");
        let missing = Path::new("/missing.wav");
        let synchronous = harness
            .service()
            .transcription(&request, AudioFile::Path(missing))
            .await;
        let Err(sync_error) = synchronous else {
            panic!("the model does not resolve");
        };
        assert!(matches!(sync_error, TranscriptionError::Resolution(_)));
        let asynchronous = harness
            .service()
            .atranscription(
                Some("no-such-provider/model-x"),
                &request,
                AudioFile::Path(missing),
                true,
            )
            .await;
        let Err(refusal) = asynchronous else {
            panic!("the model does not resolve");
        };
        let AsyncTranscriptionError::Exception(exception) = refusal else {
            panic!("the resolution failure maps to an exception");
        };
        assert!(matches!(exception.class, ErrorClass::BadRequest(_)));
        assert_eq!(exception.class.metadata().status_code, 400_u16);
        assert!(harness.dispatch.targets().is_empty());
    });
}

// litellm.openai_key is the OpenAI branch's key when no call key and no litellm.api_key are set
#[test]
fn openai_key_fallback_applies_alone() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.settings.openai_key = Some(secret("openai-key"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        harness
            .service()
            .transcription(&request_for("openai/whisper-1"), AudioFile::Bytes(&wav(1)))
            .await
            .expect("the openai transcription succeeds");
        let target = harness.dispatch.last();
        assert_eq!(target.api_key.as_deref(), Some("openai-key"));
        assert_eq!(
            target.api_base.as_deref(),
            Some("https://api.openai.com/v1")
        );
    });
}

// OPENAI_API_BASE alone provides the OpenAI base, after the missing call and litellm values
#[test]
fn openai_api_base_fallback_applies_alone() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.settings.openai_api_base = Some(String::from("https://openai-api-base.test/v1"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        harness
            .service()
            .transcription(&request_for("openai/whisper-1"), AudioFile::Bytes(&wav(1)))
            .await
            .expect("the openai transcription succeeds");
        assert_eq!(
            harness.dispatch.last().api_base.as_deref(),
            Some("https://openai-api-base.test/v1")
        );
    });
}

// litellm.azure_key is the Azure branch's key when no call key and no litellm.api_key are set
#[test]
fn azure_key_fallback_applies_alone() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.settings.azure_key = Some(secret("azure-key"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        harness
            .service()
            .transcription(&request_for("azure/whisper"), AudioFile::Bytes(&wav(1)))
            .await
            .expect("the azure transcription succeeds");
        let target = harness.dispatch.last();
        assert_eq!(target.kind, TargetKind::Azure);
        assert_eq!(target.api_key.as_deref(), Some("azure-key"));
    });
}

// AZURE_API_KEY fills the Azure key when litellm.azure_key is unset, as the chain's last value
#[test]
fn azure_api_key_environment_fills_the_key_alone() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.settings.azure_api_key = Some(secret("env-azure-key"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        harness
            .service()
            .transcription(&request_for("azure/whisper"), AudioFile::Bytes(&wav(1)))
            .await
            .expect("the azure transcription succeeds");
        assert_eq!(
            harness.dispatch.last().api_key.as_deref(),
            Some("env-azure-key")
        );
    });
}

// a single AZURE_API_BASE, AZURE_API_VERSION or litellm.api_version reaches the Azure target alone
#[test]
fn azure_base_and_version_apply_alone() {
    runtime().block_on(async {
        let mut base = Harness::new();
        base.settings.azure_api_base = Some(String::from("https://azure-api-base.test"));
        base.settings.azure_api_version = Some(String::from("2025-01-01"));
        base.dispatch.reply(response(r#"{"text":"Hello"}"#));
        base.service()
            .transcription(&request_for("azure/whisper"), AudioFile::Bytes(&wav(1)))
            .await
            .expect("the azure transcription succeeds");
        let target = base.dispatch.last();
        assert_eq!(
            target.api_base.as_deref(),
            Some("https://azure-api-base.test")
        );
        assert_eq!(target.api_version.as_deref(), Some("2025-01-01"));
        let mut version = Harness::new();
        version.settings.api_version = Some(String::from("2023-09-01"));
        version.dispatch.reply(response(r#"{"text":"Hello"}"#));
        version
            .service()
            .transcription(&request_for("azure/whisper"), AudioFile::Bytes(&wav(1)))
            .await
            .expect("the azure transcription succeeds");
        assert_eq!(
            version.dispatch.last().api_version.as_deref(),
            Some("2023-09-01")
        );
    });
}

// the Azure call's own api_base and api_key beat litellm.api_base and litellm.api_key
#[test]
fn azure_call_credentials_beat_the_globals() {
    runtime().block_on(async {
        let mut harness = Harness::new();
        harness.settings.api_base = Some(String::from("https://litellm-base.test"));
        harness.settings.api_key = Some(secret("litellm-key"));
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut azure = request_for("azure/whisper");
        azure.api_base = Some(String::from("https://call.test"));
        azure.api_key = Some(secret("call-key"));
        harness
            .service()
            .transcription(&azure, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the azure transcription succeeds");
        let target = harness.dispatch.last();
        assert_eq!(target.api_base.as_deref(), Some("https://call.test"));
        assert_eq!(target.api_key.as_deref(), Some("call-key"));
    });
}

// every openai-compatible provider joins "openai" in the transport set
#[test]
fn compatible_providers_use_the_openai_transport() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let request = request_for("groq/whisper-large-v3");
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the compatible provider uses the openai transport");
        assert_eq!(harness.dispatch.last().kind, TargetKind::OpenAi);
    });
}

// an openai gpt-4o model selects the gpt family, where whisper's models select their own
#[test]
fn gpt_transcription_selects_the_gpt_family() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let request = request_for("openai/gpt-4o-transcribe");
        harness
            .service()
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the gpt transcription succeeds");
        assert_eq!(
            harness.dispatch.last().family,
            Some(TranscriptionFamily::Gpt)
        );
    });
}

// a number past i64::MAX is still an int, since Python's int has no width
#[test]
fn error_type_name_reads_a_wide_integer_as_int() {
    assert_eq!(
        error_type_name(&json("18446744073709551615")),
        "<class 'int'>"
    );
}

// the openai and azure transports pass the caller's SDK client on unchanged
#[test]
fn sdk_clients_reach_the_openai_and_azure_transports() {
    runtime().block_on(async {
        let harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut openai = request_for("openai/whisper-1");
        openai.client = Some(SuppliedClientKind::AsyncOpenAi);
        harness
            .service()
            .transcription(&openai, AudioFile::Bytes(&audio))
            .await
            .expect("the openai transcription succeeds");
        assert_eq!(
            harness.dispatch.last().client,
            Some(SuppliedClientKind::AsyncOpenAi)
        );
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut azure = request_for("azure/whisper");
        azure.client = Some(SuppliedClientKind::AsyncAzureOpenAi);
        harness
            .service()
            .transcription(&azure, AudioFile::Bytes(&audio))
            .await
            .expect("the azure transcription succeeds");
        assert_eq!(
            harness.dispatch.last().client,
            Some(SuppliedClientKind::AsyncAzureOpenAi)
        );
    });
}

// riva's and bedrock's operations take no client, so a supplied HTTP client never reaches them
#[test]
fn riva_and_bedrock_ignore_the_caller_client() {
    runtime().block_on(async {
        let harness = Harness::new();
        let audio = wav(1);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut riva = request_for("nvidia_riva/stt");
        riva.client = Some(SuppliedClientKind::AsyncHttpHandler);
        harness
            .service()
            .transcription(&riva, AudioFile::Bytes(&audio))
            .await
            .expect("the riva transcription succeeds");
        assert_eq!(harness.dispatch.last().client, None);
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut bedrock = request_for("bedrock/stt");
        bedrock.client = Some(SuppliedClientKind::AsyncHttpHandler);
        harness
            .service()
            .transcription(&bedrock, AudioFile::Bytes(&audio))
            .await
            .expect("the bedrock transcription succeeds");
        assert_eq!(harness.dispatch.last().client, None);
    });
}

// LiteLLM's xai audio config supports language only and copies it into optional_params, leaving
// extra_body to the transform, so a fixture dispatch supplies that config to the native branch
struct XaiParams;

const XAI_SUPPORTED: [&str; 1] = ["language"];

impl OptionalParamsMapper for XaiParams {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        XAI_SUPPORTED.iter().copied().map(String::from).collect()
    }

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        _model: &str,
        _drop_params: bool,
    ) -> Result<Params, MapperError> {
        for name in XAI_SUPPORTED {
            if let Some(value) = non_default_params.get(name) {
                optional_params.insert(String::from(name), value.clone());
            }
        }
        Ok(optional_params)
    }
}

static XAI_PARAMS: XaiParams = XaiParams;

// the recorder behind a dispatch that serves the xai config above
struct XaiDispatch<'recorder>(&'recorder Recorder);

impl TranscriptionDispatch for XaiDispatch<'_> {
    type Error = FakeError;

    fn transcribe(
        &self,
        target: TranscriptionTarget<'_>,
    ) -> impl Future<Output = Result<TranscriptionAnswer, FakeError>> + Send {
        self.0.transcribe(target)
    }

    fn mapper(
        &self,
        _provider: &str,
        _config: Option<AudioTranscriptionConfigTag>,
    ) -> Option<&dyn OptionalParamsMapper> {
        Some(&XAI_PARAMS)
    }
}

// a native destination keeps the mapper's named options beside the nested extra_body, the shape
// LiteLLM's xai transform flattens with the direct option winning
#[test]
fn native_keeps_the_named_options_beside_the_nested_extra_body() {
    runtime().block_on(async {
        let harness = Harness::new();
        harness.dispatch.reply(response(r#"{"text":"Hello"}"#));
        let mut request = request_for("xai/whisper-1");
        request.language = Some(String::from("en"));
        request.params = params(r#"{"extra_body":{"language":"fr"},"custom_field":7}"#);
        let mapped = XaiDispatch(&harness.dispatch);
        let template = harness.service();
        let service = Transcriptions {
            dispatch: &mapped,
            settings: template.settings,
            resolution: template.resolution,
            bundled: template.bundled,
            files: template.files,
            decoder: template.decoder,
            redactor: template.redactor,
            logging: template.logging,
            identify: template.identify,
        };
        service
            .transcription(&request, AudioFile::Bytes(&wav(1)))
            .await
            .expect("the native transcription succeeds");
        let recorded = harness.dispatch.last();
        assert_eq!(recorded.kind, TargetKind::Native);
        assert_eq!(
            recorded.optional_params.get("language"),
            Some(&value(r#""en""#))
        );
        assert_eq!(
            recorded.optional_params.get("extra_body"),
            Some(&value(r#"{"language":"fr","custom_field":7}"#))
        );
    });
}
