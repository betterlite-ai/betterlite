use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::api::file_http_upload::service::Mode;
use crate::api::file_provider_write::{FileWriteError, LoggerSeed, SuppliedClient};
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

use super::{
    CALL_ID, Credentials, Logged, Presence, Recorder, Rejected, Request, Seen, delete,
    delete_input, sent_data, started, upload, upload_input, write_with,
};

const DEFAULT_TIMEOUT: FineTuningTimeout = FineTuningTimeout::Seconds(600.0_f64);

fn secret(text: &str) -> Option<SecretString> {
    Some(SecretString::from(text.to_owned()))
}

fn configured() -> ConfiguredSettings {
    ConfiguredSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: secret("sk-env"),
        openai_organization: Some("org-env".to_owned()),
        azure_api_base: Some("https://azure.example".to_owned()),
        azure_api_version: Some("2024-06-01".to_owned()),
        azure_openai_api_key: None,
        azure_api_key: secret("az-env"),
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn handler() -> SuppliedClient<&'static str> {
    SuppliedClient::Handler {
        mode: Mode::Sync,
        client: "httpx-handler",
    }
}

// which of the two SDK clouds a dispatch test reads its credentials for
#[derive(Debug, Clone, Copy)]
enum Cloud {
    OpenAi,
    Azure,
}

// value is the request's organization for OpenAI and its api_version for Azure
fn sdk_credentials(cloud: Cloud, mode: Mode, value: &str) -> Credentials {
    let (api_base, api_key, api_version, organization, max_retries) = match cloud {
        Cloud::OpenAi => (
            "https://api.openai.com/v1",
            "sk-env",
            None,
            Some(value),
            Some(1),
        ),
        Cloud::Azure => (
            "https://azure.example",
            "az-request",
            Some(value),
            None,
            Some(3),
        ),
    };
    Credentials {
        mode,
        api_base: Some(api_base.to_owned()),
        api_key: Some(api_key.to_owned()),
        api_version: api_version.map(str::to_owned),
        organization: organization.map(str::to_owned),
        timeout: DEFAULT_TIMEOUT,
        max_retries,
    }
}

#[test]
fn config_first_upload_takes_the_files_config() {
    let mut input = upload_input(Presence::Value("anthropic"));
    input.extra_headers = Some(IndexMap::from([("X-Trace".to_owned(), "t1".to_owned())]));
    input.params.api_base = Some("https://anthropic.example".to_owned());
    input.params.api_key = secret("sk-ant");
    input.params.client = Some(handler());
    let (_, seen) = upload(Mode::Asynchronous, input);
    let [
        Seen::ConfigUpload {
            config,
            mode,
            headers,
            api_base,
            api_key,
            logger,
            client,
            timeout,
            ..
        },
    ] = seen.as_slice()
    else {
        panic!("expected the config upload, saw {seen:?}");
    };
    assert_eq!(*config, ProviderId::Anthropic);
    assert_eq!(*mode, Mode::Asynchronous);
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("t1"));
    assert_eq!(api_base.as_deref(), Some("https://anthropic.example"));
    assert_eq!(api_key.as_deref(), Some("sk-ant"));
    assert_eq!(logger, "caller-logger");
    assert_eq!(client.as_ref(), Some(&handler()));
    assert_eq!(*timeout, DEFAULT_TIMEOUT);
}

// a provider SDK client is not an httpx handler, so the generic paths drop it
#[test]
fn generic_paths_drop_an_sdk_client() {
    let mut uploading = upload_input(Presence::Value("anthropic"));
    uploading.params.client = Some(SuppliedClient::Sdk("sdk"));
    let (_, uploaded) = upload(Mode::Sync, uploading);
    assert!(matches!(
        uploaded.as_slice(),
        [Seen::ConfigUpload { client: None, .. }]
    ));
    let mut deleting = delete_input(Some("anthropic"));
    deleting.params.client = Some(SuppliedClient::Sdk("sdk"));
    let (_, deleted) = delete(Mode::Sync, deleting);
    assert!(matches!(
        deleted.as_slice(),
        [Seen::ConfigDelete { client: None, .. }]
    ));
}

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
fn write_provider_dispatch(#[case] provider: &str) {
    let recorder = Recorder::default();
    let mut uploading = upload_input(Presence::Value(provider));
    uploading.params.organization = Some("org-request".to_owned());
    uploading.params.max_retries = Some(1);
    uploading.params.client = Some(SuppliedClient::Sdk("ignored"));
    let upload_request = Request::Upload(uploading);
    write_with(&recorder, &configured(), Mode::Sync, upload_request)
        .file()
        .unwrap();
    let mut deleting = delete_input(Some(provider));
    deleting.params.max_retries = Some(1);
    deleting.params.client = Some(SuppliedClient::Sdk("ignored"));
    let delete_request = Request::Delete(deleting);
    write_with(&recorder, &configured(), Mode::Asynchronous, delete_request)
        .deleted()
        .unwrap();
    let expected = vec![
        Seen::OpenAiUpload(
            sdk_credentials(Cloud::OpenAi, Mode::Sync, "org-request"),
            sent_data(&recorder.seen()[..1]),
        ),
        Seen::OpenAiDelete(
            sdk_credentials(Cloud::OpenAi, Mode::Asynchronous, "org-env"),
            "file-abc".to_owned(),
        ),
    ];
    assert_eq!(recorder.seen(), expected);
}

#[test]
fn azure_receives_version_retries_and_its_delete_client() {
    let recorder = Recorder::default();
    let mut uploading = upload_input(Presence::Value("azure"));
    uploading.params.api_key = secret("az-request");
    uploading.params.api_version = Some("2024-10-21".to_owned());
    uploading.params.max_retries = Some(3);
    let sent = write_with(
        &recorder,
        &configured(),
        Mode::Sync,
        Request::Upload(uploading),
    )
    .file();
    assert_eq!(sent.unwrap().id, "file-abc");
    let mut deleting = delete_input(Some("azure"));
    deleting.params.api_key = secret("az-request");
    deleting.params.max_retries = Some(3);
    deleting.params.client = Some(SuppliedClient::Sdk("azure-client"));
    write_with(
        &recorder,
        &configured(),
        Mode::Sync,
        Request::Delete(deleting),
    )
    .deleted()
    .unwrap();
    let seen = recorder.seen();
    assert!(
        matches!(seen.first(), Some(Seen::AzureUpload(call, _)) if *call == sdk_credentials(Cloud::Azure, Mode::Sync, "2024-10-21"))
    );
    let Some(Seen::AzureDelete {
        call,
        file_id,
        client,
        api_key,
        api_base,
    }) = seen.get(1)
    else {
        panic!("expected the azure delete, saw {seen:?}");
    };
    assert_eq!(
        *call,
        sdk_credentials(Cloud::Azure, Mode::Sync, "2024-06-01")
    );
    assert_eq!(file_id, "file-abc");
    assert_eq!(*client, Some(SuppliedClient::Sdk("azure-client")));
    assert_eq!(api_key.as_deref(), Some("az-request"));
    assert_eq!(*api_base, None);
}

// the generic delete copies the trusted snapshot and assigns api_key and api_base even when they are null
#[test]
fn generic_delete_assigns_credentials_even_when_null() {
    let mut input = delete_input(Some("anthropic"));
    input.extra_body = Some(IndexMap::from([("ignored".to_owned(), "x".to_owned())]));
    input.extra_headers = Some(IndexMap::from([("X-Trace".to_owned(), "t2".to_owned())]));
    input.params.client = Some(handler());
    input.params.credentials = Some(TrustedCredentials {
        values: BTreeMap::from([("aws_bucket".to_owned(), SecretString::from("b"))]),
    });
    let (sent, seen) = delete(Mode::Sync, input);
    assert_eq!(sent.unwrap().id, "file-abc");
    let [
        Seen::ConfigDelete {
            api_key,
            api_base,
            trusted,
            headers,
            client,
            ..
        },
    ] = seen.as_slice()
    else {
        panic!("expected the config delete, saw {seen:?}");
    };
    assert_eq!((api_key, api_base), (&None, &None));
    assert_eq!(
        trusted.as_deref(),
        Some(["aws_bucket".to_owned()].as_slice())
    );
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("t2"));
    assert_eq!(client.as_ref(), Some(&handler()));
}

#[test]
fn provider_failures_pass_through_unchanged() {
    let failure = Rejected {
        status: 404,
        message: "No such File object: file-abc".to_owned(),
    };
    let recorder = Recorder {
        seen: Arc::default(),
        failure: Some(failure.clone()),
    };
    let upload_request = Request::Upload(upload_input(Presence::Value("anthropic")));
    let uploaded = write_with(&recorder, &configured(), Mode::Sync, upload_request).file();
    assert!(matches!(uploaded, Err(FileWriteError::Provider(ref seen)) if *seen == failure));
    let delete_request = Request::Delete(delete_input(None));
    let deleted = write_with(&recorder, &configured(), Mode::Sync, delete_request).deleted();
    assert!(matches!(deleted, Err(FileWriteError::Provider(ref seen)) if *seen == failure));
}

#[rstest]
#[case::hint_names_the_provider(
    Some("anthropic/claude-3-5-sonnet-20240620"),
    "",
    Some(ProviderId::Anthropic)
)]
#[case::declared_provider_stays(Some("gpt-4o"), "anthropic", Some(ProviderId::Anthropic))]
#[case::no_hint(None, "anthropic", Some(ProviderId::Anthropic))]
fn delete_model_and_logger(
    #[case] model: Option<&str>,
    #[case] provider: &str,
    #[case] expected: Option<ProviderId>,
) {
    let mut input = delete_input(Some(provider));
    input.model = model.map(str::to_owned);
    let (sent, seen) = delete(Mode::Sync, input);
    assert!(!sent.unwrap().deleted);
    let found = match seen.as_slice() {
        [Seen::ConfigDelete { config, logger, .. }] => {
            assert_eq!(*logger, Logged::Existing("caller-logger".to_owned()));
            Some(*config)
        }
        _ => None,
    };
    assert_eq!(found, expected);
}

// a hint that does not resolve is swallowed, so the declared provider is converted and rejected as before
#[test]
fn unresolved_hint_keeps_the_declared_provider() {
    let mut input = delete_input(Some(""));
    input.model = Some("not-a-known-model".to_owned());
    let (sent, seen) = delete(Mode::Sync, input);
    assert_eq!(
        sent.unwrap_err().to_string(),
        "'' is not a valid LlmProviders"
    );
    assert!(seen.is_empty());
}

// mode, the litellm_call_id the case sends, its id, the logger's call_type, call id and function id
type LoggerCase = (
    Mode,
    Presence<'static>,
    Option<&'static str>,
    &'static str,
    Option<&'static str>,
    &'static str,
);

#[rstest]
#[case::sync_absent_id((Mode::Sync, Presence::Absent, None, "file_delete", Some(CALL_ID), ""))]
#[case::async_absent_id((
    Mode::Asynchronous,
    Presence::Absent,
    None,
    "afile_delete",
    Some(CALL_ID),
    ""
))]
#[case::explicit_null_id((Mode::Sync, Presence::Null, Some("fn-7"), "file_delete", None, "fn-7"))]
#[case::given_id((
    Mode::Asynchronous,
    Presence::Value("call-given"),
    Some(""),
    "afile_delete",
    Some("call-given"),
    ""
))]
fn missing_logger_is_created(#[case] case: LoggerCase) {
    let (mode, call_id, id, call_type, litellm_call_id, function_id) = case;
    let mut input = delete_input(Some("anthropic"));
    input.params.logger = None;
    input.params.litellm_call_id = match call_id {
        Presence::Absent => None,
        Presence::Null => Some(None),
        Presence::Value(given) => Some(Some(given.to_owned())),
    };
    input.params.id = id.map(str::to_owned);
    let (_, seen) = delete(mode, input);
    let expected = Logged::Created(LoggerSeed {
        call_type,
        start_time: started(),
        litellm_call_id: litellm_call_id.map(str::to_owned),
        function_id: function_id.to_owned(),
    });
    assert!(matches!(seen.as_slice(), [Seen::ConfigDelete { logger, .. }] if *logger == expected));
}
