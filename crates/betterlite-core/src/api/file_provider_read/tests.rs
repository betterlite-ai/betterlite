#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{FileKind, FileObject, FilePage, FilesPurpose, ListKind};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use uuid::Uuid;

use crate::api::fine_tuning_read::{ReadStructuredTimeout, ReadTimeout, ReadTimeoutValue};
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

use super::error::{
    FileReadError, ReadFunction, UNSUPPORTED_BODY, UNSUPPORTED_MODEL, UNSUPPORTED_URL,
};
use super::target::{
    FileListing, FileReadDispatch, NewLogger, ReadAnswer, ReadLogger, ReadMode, ReadOperation,
    ReadTarget,
};
use super::{
    CallerClient, FileProviderRead, Kwarg, ReadContext, ReadEntry, ReadKwargs, ReadProvider,
};

const STARTED: &str = "2026-01-01T00:00:00+00:00";
const GENERATED_ID: &str = "00000000-0000-0000-0000-000000000001";
const DEFAULT_MODE: ReadMode = ReadMode {
    is_async: false,
    timeout: FineTuningTimeout::Seconds(600.0_f64),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FakeConfig(ProviderId);

#[derive(Debug, Clone, PartialEq)]
struct GenericSeen {
    config: FakeConfig,
    api_key: Option<String>,
    api_base: Option<String>,
    trusted: Option<Vec<String>>,
    headers: BTreeMap<String, String>,
    created: Option<NewLogger>,
    client: Option<ClientKind>,
    mode: ReadMode,
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Compatible {
        api_base: String,
        api_key: Option<String>,
        organization: Option<String>,
        max_retries: Option<i64>,
        mode: ReadMode,
    },
    Azure {
        api_base: Option<String>,
        api_key: Option<String>,
        api_version: Option<String>,
        max_retries: Option<i64>,
        mode: ReadMode,
    },
    Generic(Box<GenericSeen>),
}

impl Seen {
    const fn mode(&self) -> ReadMode {
        match self {
            Self::Compatible { mode, .. } | Self::Azure { mode, .. } => *mode,
            Self::Generic(generic) => generic.mode,
        }
    }

    fn generic(&self) -> GenericSeen {
        match self {
            Self::Generic(generic) => (**generic).clone(),
            Self::Compatible { .. } | Self::Azure { .. } => panic!("not a generic read: {self:?}"),
        }
    }
}

// The file id or purpose the dispatch was handed beside the target
#[derive(Debug, Clone, PartialEq)]
struct Sent {
    target: Seen,
    argument: Option<String>,
}

struct FakeDispatch {
    configured: Vec<ProviderId>,
    sent: Mutex<Vec<Sent>>,
    listing: FileListing,
}

fn seen(target: ReadTarget<'_, FakeConfig, ()>) -> Seen {
    match target {
        ReadTarget::Compatible(read) => Seen::Compatible {
            api_key: read
                .api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
            api_base: read.api_base,
            organization: read.organization,
            max_retries: read.max_retries,
            mode: read.mode,
        },
        ReadTarget::Azure(read) => Seen::Azure {
            api_key: read
                .api_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
            api_base: read.api_base,
            api_version: read.api_version,
            max_retries: read.max_retries,
            mode: read.mode,
        },
        ReadTarget::Generic(boxed) => {
            let read = *boxed;
            Seen::Generic(Box::new(GenericSeen {
                config: read.config,
                api_key: read
                    .api_key
                    .as_ref()
                    .map(|secret| secret.expose_secret().to_owned()),
                api_base: read.api_base,
                trusted: read
                    .trusted
                    .map(|snapshot| snapshot.values.keys().cloned().collect()),
                headers: read.headers,
                created: match read.logger {
                    ReadLogger::Created(logger) => Some(*logger),
                    ReadLogger::Supplied(_) => None,
                },
                client: read.client.map(|supplied| supplied.kind),
                mode: read.mode,
            }))
        }
    }
}

fn file_object(id: &str) -> FileObject {
    FileObject {
        id: id.to_owned(),
        bytes: 3,
        created_at: 1_700_000_000,
        filename: "batch.jsonl".to_owned(),
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: None,
        expires_at: None,
        status_details: None,
    }
}

// the file or listing the dispatch answered, which each test unwraps once
fn answered_file(answer: ReadAnswer) -> FileObject {
    match answer {
        ReadAnswer::File(file) => *file,
        ReadAnswer::Listing(_) => panic!("not a file answer"),
    }
}

fn answered_listing(answer: ReadAnswer) -> FileListing {
    match answer {
        ReadAnswer::Listing(listing) => listing,
        ReadAnswer::File(_) => panic!("not a listing answer"),
    }
}

impl FileReadDispatch for FakeDispatch {
    type Config = FakeConfig;
    type Client = ();
    type Error = Infallible;

    fn files_config(&self, provider: ProviderId) -> Option<FakeConfig> {
        self.configured
            .contains(&provider)
            .then_some(FakeConfig(provider))
    }

    fn read(
        &self,
        target: ReadTarget<'_, FakeConfig, ()>,
        operation: ReadOperation<'_>,
    ) -> impl Future<Output = Result<ReadAnswer, Infallible>> + Send {
        let (argument, answer) = match operation {
            ReadOperation::Retrieve { file_id } => (
                Some(file_id.to_owned()),
                ReadAnswer::File(Box::new(file_object(file_id))),
            ),
            ReadOperation::List { purpose } => (
                purpose.map(str::to_owned),
                ReadAnswer::Listing(self.listing.clone()),
            ),
        };
        self.record(target, argument);
        ready(Ok(answer))
    }
}

impl FakeDispatch {
    // every dispatch keeps the target it was handed and that call's one argument
    fn record(&self, target: ReadTarget<'_, FakeConfig, ()>, argument: Option<String>) {
        let sent = Sent {
            target: seen(target),
            argument,
        };
        self.sent.lock().unwrap().push(sent);
    }
}

fn service(settings: Settings, listing: FileListing) -> FileProviderRead<FakeDispatch> {
    FileProviderRead {
        dispatch: FakeDispatch {
            configured: vec![ProviderId::Anthropic, ProviderId::Bedrock],
            sent: Mutex::new(Vec::new()),
            listing,
        },
        settings,
        clock: || DateTime::parse_from_rfc3339(STARTED).unwrap(),
        call_ids: || Uuid::from_u128(1),
    }
}

fn reader() -> FileProviderRead<FakeDispatch> {
    service(Settings::default(), FileListing::Files(Vec::new()))
}

fn kwargs(provider: &str) -> ReadKwargs {
    ReadKwargs {
        custom_llm_provider: ReadProvider(provider.to_owned()),
        ..ReadKwargs::default()
    }
}

const fn bare() -> ReadContext<'static, ()> {
    ReadContext {
        trusted: None,
        client: None,
        logger: None,
    }
}

fn sent(read: &FileProviderRead<FakeDispatch>) -> Vec<Sent> {
    read.dispatch.sent.lock().unwrap().clone()
}

fn text_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

async fn retrieved(read: &FileProviderRead<FakeDispatch>, kwargs: &ReadKwargs) -> Seen {
    read.run(
        ReadEntry::Sync,
        ReadOperation::Retrieve { file_id: "file-1" },
        kwargs,
        bare(),
    )
    .await
    .unwrap();
    sent(read).pop().unwrap().target
}

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[tokio::test]
async fn retrieve_provider_metadata_compatible(#[case] provider: &str) {
    let settings = Settings {
        api_key: Some(SecretString::from("sk-settings")),
        ..Settings::default()
    };
    let read = service(settings, FileListing::Files(Vec::new()));
    let mut call = kwargs(provider);
    call.extra_headers = Some(text_map(&[("x-dropped", "1")]));
    call.extra_body = Some(text_map(&[("dropped", "1")]));
    call.max_retries = Some(4);
    let answer = read
        .run(
            ReadEntry::Sync,
            ReadOperation::Retrieve {
                file_id: "a b/c%2F",
            },
            &call,
            bare(),
        )
        .await
        .unwrap();
    assert_eq!(answered_file(answer), file_object("a b/c%2F"));
    let expected = Sent {
        target: Seen::Compatible {
            api_base: "https://api.openai.com/v1".to_owned(),
            api_key: Some("sk-settings".to_owned()),
            organization: None,
            max_retries: Some(4),
            mode: DEFAULT_MODE,
        },
        argument: Some("a b/c%2F".to_owned()),
    };
    assert_eq!(sent(&read), vec![expected]);
}

#[tokio::test]
async fn retrieve_provider_metadata_defaults_to_openai() {
    let read = reader();
    let target = retrieved(&read, &ReadKwargs::default()).await;
    assert!(matches!(target, Seen::Compatible { api_key: None, .. }));
}

#[tokio::test]
async fn retrieve_provider_metadata_azure() {
    let settings = Settings {
        azure_api_version: Some("2024-06-01".to_owned()),
        azure_api_key: Some(SecretString::from("sk-env-azure")),
        ..Settings::default()
    };
    let read = service(settings, FileListing::Files(Vec::new()));
    let mut call = kwargs("azure");
    call.api_base = Some("https://azure.example".to_owned());
    call.organization = Some("org-ignored".to_owned());
    call.extra_headers = Some(text_map(&[("x-dropped", "1")]));
    let expected = Seen::Azure {
        api_base: Some("https://azure.example".to_owned()),
        api_key: Some("sk-env-azure".to_owned()),
        api_version: Some("2024-06-01".to_owned()),
        max_retries: None,
        mode: DEFAULT_MODE,
    };
    assert_eq!(retrieved(&read, &call).await, expected);
}

#[rstest]
#[case::with_headers(Some(text_map(&[("x-trace", "1")])), text_map(&[("x-trace", "1")]))]
#[case::without_headers(None, BTreeMap::new())]
#[tokio::test]
async fn retrieve_provider_metadata_generic(
    #[case] extra_headers: Option<BTreeMap<String, String>>,
    #[case] headers: BTreeMap<String, String>,
) {
    let read = reader();
    let mut call = kwargs("anthropic");
    call.extra_headers = extra_headers;
    call.extra_body = Some(text_map(&[("dropped", "1")]));
    let generic = retrieved(&read, &call).await.generic();
    assert_eq!(generic.config, FakeConfig(ProviderId::Anthropic));
    assert_eq!(generic.headers, headers);
}

#[rstest]
#[case::retrieve_checks_compatible_names_first(ReadFunction::FileRetrieve, false)]
#[case::list_asks_for_a_config_first(ReadFunction::FileList, true)]
#[tokio::test]
async fn retrieve_provider_metadata_branch_order(
    #[case] function: ReadFunction,
    #[case] generic: bool,
) {
    let mut read = reader();
    read.dispatch.configured.push(ProviderId::Openai);
    let call = kwargs("openai");
    match function {
        ReadFunction::FileRetrieve => {
            read.run(
                ReadEntry::Sync,
                ReadOperation::Retrieve { file_id: "file-1" },
                &call,
                bare(),
            )
            .await
            .unwrap();
        }
        ReadFunction::FileList => {
            read.run(
                ReadEntry::Sync,
                ReadOperation::List { purpose: None },
                &call,
                bare(),
            )
            .await
            .unwrap();
        }
    }
    let target = sent(&read).pop().unwrap().target;
    assert_eq!(matches!(target, Seen::Generic(_)), generic);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::batch(Some("batch"))]
#[tokio::test]
async fn list_purpose_without_client_cursor(#[case] purpose: Option<&str>) {
    let page = FilePage {
        object: Some(ListKind::List),
        data: vec![file_object("b"), file_object("a"), file_object("b")],
        has_more: Some(false),
        first_id: None,
        last_id: None,
    };
    let read = service(Settings::default(), FileListing::Page(Box::new(page)));
    let mut call = kwargs("openai");
    call.after = Some("f".to_owned());
    call.limit = Some(1);
    call.order = Some("asc".to_owned());
    let listing = answered_listing(
        read.run(
            ReadEntry::Sync,
            ReadOperation::List { purpose },
            &call,
            bare(),
        )
        .await
        .unwrap(),
    );
    assert_eq!(listing, read.dispatch.listing);
    let expected = Sent {
        target: Seen::Compatible {
            api_base: "https://api.openai.com/v1".to_owned(),
            api_key: None,
            organization: None,
            max_retries: None,
            mode: DEFAULT_MODE,
        },
        argument: purpose.map(str::to_owned),
    };
    assert_eq!(sent(&read), vec![expected]);
}

#[tokio::test]
async fn list_purpose_without_client_cursor_generic_files() {
    let files = vec![file_object("b"), file_object("a"), file_object("b")];
    let read = service(Settings::default(), FileListing::Files(files.clone()));
    let mut call = kwargs("anthropic");
    call.after = Some("f".to_owned());
    let listing = answered_listing(
        read.run(
            ReadEntry::Sync,
            ReadOperation::List {
                purpose: Some("batch"),
            },
            &call,
            bare(),
        )
        .await
        .unwrap(),
    );
    assert_eq!(listing, FileListing::Files(files));
    let last = sent(&read).pop().unwrap();
    assert_eq!(last.argument.as_deref(), Some("batch"));
    assert_eq!(last.target.generic().headers, BTreeMap::new());
}

#[tokio::test]
async fn list_purpose_without_client_cursor_azure() {
    let read = reader();
    read.run(
        ReadEntry::Sync,
        ReadOperation::List { purpose: None },
        &kwargs("azure"),
        bare(),
    )
    .await
    .unwrap();
    let target = sent(&read).pop().unwrap().target;
    assert!(matches!(target, Seen::Azure { api_base: None, .. }));
}

fn number(value: u32) -> ReadTimeoutValue {
    ReadTimeoutValue::Number(Number::from(value))
}

fn text(value: &str) -> ReadTimeoutValue {
    ReadTimeoutValue::Text(value.to_owned())
}

fn phases(read: Option<u32>) -> ReadTimeout {
    ReadTimeout::Structured(ReadStructuredTimeout {
        connect: None,
        read: read.map(Number::from),
        write: None,
        pool: None,
    })
}

const fn kept(read: Option<f64>) -> FineTuningTimeout {
    FineTuningTimeout::Structured(StructuredTimeout {
        connect: None,
        read,
        write: None,
        pool: None,
    })
}

#[rstest]
#[case::timeout_over_request_timeout(
    Some(ReadTimeout::Plain(number(2))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(2.0_f64)
)]
#[case::request_timeout_alone(None, Some(number(3)), "openai", FineTuningTimeout::Seconds(3.0_f64))]
#[case::both_absent(None, None, "openai", FineTuningTimeout::Seconds(600.0_f64))]
#[case::zero_falls_through(
    Some(ReadTimeout::Plain(number(0))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::zero_alone(
    Some(ReadTimeout::Plain(number(0))),
    None,
    "openai",
    FineTuningTimeout::Seconds(600.0_f64)
)]
#[case::zero_request_timeout(
    None,
    Some(number(0)),
    "openai",
    FineTuningTimeout::Seconds(600.0_f64)
)]
#[case::empty_text_falls_through(
    Some(ReadTimeout::Plain(text(""))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::numeric_text(
    Some(ReadTimeout::Plain(text("2.5"))),
    None,
    "openai",
    FineTuningTimeout::Seconds(2.5_f64)
)]
#[case::request_timeout_text(
    None,
    Some(text("4.5")),
    "anthropic",
    FineTuningTimeout::Seconds(4.5_f64)
)]
#[case::openai_keeps_phases(Some(phases(Some(5))), Some(number(3)), "openai", kept(Some(5.0_f64)))]
#[case::azure_keeps_phases(Some(phases(Some(5))), None, "azure", kept(Some(5.0_f64)))]
#[case::bedrock_keeps_phases(Some(phases(Some(5))), None, "bedrock", kept(Some(5.0_f64)))]
#[case::anthropic_reads_the_read_phase(
    Some(phases(Some(5))),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(5.0_f64)
)]
#[case::anthropic_without_read_phase(
    Some(phases(None)),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(600.0_f64)
)]
#[tokio::test]
async fn read_deadline_and_mode_timeout(
    #[case] timeout: Option<ReadTimeout>,
    #[case] request_timeout: Option<ReadTimeoutValue>,
    #[case] provider: &str,
    #[case] expected: FineTuningTimeout,
) {
    let read = reader();
    let mut call = kwargs(provider);
    call.timeout = timeout;
    call.request_timeout = request_timeout;
    assert_eq!(retrieved(&read, &call).await.mode().timeout, expected);
}

#[rstest]
#[case::openai("openai")]
#[case::unsupported_provider("cohere")]
#[case::unknown_provider("bogus")]
#[tokio::test]
async fn read_deadline_and_mode_bad_text_fails_first(#[case] provider: &str) {
    let read = reader();
    let mut call = kwargs(provider);
    call.timeout = Some(ReadTimeout::Plain(text("soon")));
    let retrieve = read
        .run(
            ReadEntry::Sync,
            ReadOperation::Retrieve { file_id: "file-1" },
            &call,
            bare(),
        )
        .await;
    let list = read
        .run(
            ReadEntry::Sync,
            ReadOperation::List { purpose: None },
            &call,
            bare(),
        )
        .await;
    assert!(matches!(retrieve, Err(FileReadError::Timeout(_))));
    assert!(matches!(list, Err(FileReadError::Timeout(_))));
    assert_eq!(retrieve.unwrap_err().status(), None);
    assert_eq!(sent(&read), Vec::new());
}

#[rstest]
#[case::sync(ReadEntry::Sync, false, false)]
#[case::sync_with_is_async(ReadEntry::Sync, true, true)]
#[case::async_wrapper(ReadEntry::Async, false, true)]
#[case::async_wrapper_with_is_async(ReadEntry::Async, true, true)]
#[tokio::test]
async fn read_deadline_and_mode_async(
    #[case] entry: ReadEntry,
    #[case] is_async: bool,
    #[case] expected: bool,
) {
    let read = reader();
    let mut call = kwargs("anthropic");
    call.is_async = is_async;
    read.run(
        entry,
        ReadOperation::Retrieve { file_id: "file-1" },
        &call,
        bare(),
    )
    .await
    .unwrap();
    read.run(entry, ReadOperation::List { purpose: None }, &call, bare())
        .await
        .unwrap();
    let modes: Vec<bool> = sent(&read)
        .iter()
        .map(|sent| sent.target.mode().is_async)
        .collect();
    assert_eq!(modes, vec![expected, expected]);
}

#[tokio::test]
async fn read_trusted_context_generic_fields() {
    let settings = Settings {
        api_key: Some(SecretString::from("sk-settings")),
        api_base: Some("https://settings.example".to_owned()),
        ..Settings::default()
    };
    let read = service(settings, FileListing::Files(Vec::new()));
    let trusted = TrustedCredentials {
        values: BTreeMap::from([("bucket".to_owned(), SecretString::from("files"))]),
    };
    let context = ReadContext {
        trusted: Some(&trusted),
        client: Some(CallerClient::Http(SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &(),
        })),
        logger: None,
    };
    read.run(
        ReadEntry::Sync,
        ReadOperation::Retrieve { file_id: "file-1" },
        &kwargs("anthropic"),
        context,
    )
    .await
    .unwrap();
    let generic = sent(&read).pop().unwrap().target.generic();
    assert_eq!(generic.trusted, Some(vec!["bucket".to_owned()]));
    assert_eq!(generic.api_key, None);
    assert_eq!(generic.api_base, None);
    assert_eq!(generic.client, Some(ClientKind::Asynchronous));
}

#[tokio::test]
async fn read_trusted_context_drops_sdk_clients() {
    let read = reader();
    let mut call = kwargs("anthropic");
    call.api_key = Some(SecretString::from("sk-call"));
    call.api_base = Some("https://call.example".to_owned());
    let context = ReadContext {
        trusted: None,
        client: Some(CallerClient::Sdk),
        logger: None,
    };
    read.run(
        ReadEntry::Sync,
        ReadOperation::List { purpose: None },
        &call,
        context,
    )
    .await
    .unwrap();
    let generic = sent(&read).pop().unwrap().target.generic();
    assert_eq!(generic.client, None);
    assert_eq!(generic.trusted, None);
    assert_eq!(generic.api_key.as_deref(), Some("sk-call"));
    assert_eq!(generic.api_base.as_deref(), Some("https://call.example"));
}

// the logging object the case expects LiteLLM to create
#[derive(Clone, Copy)]
struct ExpectedLogger {
    call_type: &'static str,
    call_id: Option<&'static str>,
    function_id: &'static str,
}

#[rstest]
#[case::retrieve(
    ReadFunction::FileRetrieve,
    ReadEntry::Sync,
    Kwarg::Absent,
    Kwarg::Absent,
    ExpectedLogger { call_type: "file_retrieve", call_id: Some(GENERATED_ID), function_id: "" }
)]
#[case::aretrieve(
    ReadFunction::FileRetrieve,
    ReadEntry::Async,
    Kwarg::Absent,
    Kwarg::Absent,
    ExpectedLogger { call_type: "afile_retrieve", call_id: Some(GENERATED_ID), function_id: "" }
)]
#[case::list(
    ReadFunction::FileList,
    ReadEntry::Sync,
    Kwarg::Absent,
    Kwarg::Absent,
    ExpectedLogger { call_type: "file_list", call_id: Some(GENERATED_ID), function_id: "" }
)]
#[case::alist(
    ReadFunction::FileList,
    ReadEntry::Async,
    Kwarg::Absent,
    Kwarg::Absent,
    ExpectedLogger { call_type: "afile_list", call_id: Some(GENERATED_ID), function_id: "" }
)]
#[case::null_call_id_stays_null(
    ReadFunction::FileRetrieve,
    ReadEntry::Sync,
    Kwarg::Null,
    Kwarg::Absent,
    ExpectedLogger { call_type: "file_retrieve", call_id: None, function_id: "" }
)]
#[case::given_call_id(
    ReadFunction::FileList,
    ReadEntry::Sync,
    Kwarg::Text("call-7".to_owned()),
    Kwarg::Absent,
    ExpectedLogger { call_type: "file_list", call_id: Some("call-7"), function_id: "" }
)]
#[case::retrieve_null_id_is_empty(
    ReadFunction::FileRetrieve,
    ReadEntry::Sync,
    Kwarg::Absent,
    Kwarg::Null,
    ExpectedLogger { call_type: "file_retrieve", call_id: Some(GENERATED_ID), function_id: "" }
)]
#[case::list_null_id_is_none_text(
    ReadFunction::FileList,
    ReadEntry::Sync,
    Kwarg::Absent,
    Kwarg::Null,
    ExpectedLogger { call_type: "file_list", call_id: Some(GENERATED_ID), function_id: "None" }
)]
#[case::retrieve_given_id(
    ReadFunction::FileRetrieve,
    ReadEntry::Sync,
    Kwarg::Absent,
    Kwarg::Text("fn-1".to_owned()),
    ExpectedLogger { call_type: "file_retrieve", call_id: Some(GENERATED_ID), function_id: "fn-1" }
)]
#[case::list_given_id(
    ReadFunction::FileList,
    ReadEntry::Sync,
    Kwarg::Absent,
    Kwarg::Text("fn-2".to_owned()),
    ExpectedLogger { call_type: "file_list", call_id: Some(GENERATED_ID), function_id: "fn-2" }
)]
#[tokio::test]
async fn read_trusted_context_creates_logger(
    #[case] function: ReadFunction,
    #[case] entry: ReadEntry,
    #[case] litellm_call_id: Kwarg,
    #[case] id: Kwarg,
    #[case] expected: ExpectedLogger,
) {
    let read = reader();
    let mut call = kwargs("anthropic");
    call.litellm_call_id = litellm_call_id;
    call.id = id;
    match function {
        ReadFunction::FileRetrieve => {
            read.run(
                entry,
                ReadOperation::Retrieve { file_id: "file-1" },
                &call,
                bare(),
            )
            .await
            .unwrap();
        }
        ReadFunction::FileList => {
            read.run(entry, ReadOperation::List { purpose: None }, &call, bare())
                .await
                .unwrap();
        }
    }
    let created = sent(&read).pop().unwrap().target.generic().created;
    let logger = NewLogger {
        call_type: expected.call_type,
        start_time: DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
        litellm_call_id: expected.call_id.map(str::to_owned),
        function_id: expected.function_id.to_owned(),
    };
    assert_eq!(created, Some(logger));
}

#[tokio::test]
async fn read_trusted_context_keeps_supplied_logger() {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"files","call_type":"file_retrieve","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap();
    let read = reader();
    let context = ReadContext {
        trusted: None,
        client: None,
        logger: Some(&mut state),
    };
    read.run(
        ReadEntry::Sync,
        ReadOperation::Retrieve { file_id: "file-1" },
        &kwargs("anthropic"),
        context,
    )
    .await
    .unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.generic().created, None);
}

#[rstest]
#[case::retrieve(
    ReadFunction::FileRetrieve,
    "LiteLLM doesn't support cohere for 'file_retrieve'. Only 'openai', 'azure', 'manus', and 'anthropic' are supported."
)]
#[case::list(
    ReadFunction::FileList,
    "LiteLLM doesn't support cohere for 'file_list'. Only 'openai', 'azure', 'manus', and 'anthropic' are supported."
)]
#[tokio::test]
async fn unsupported_file_read(#[case] function: ReadFunction, #[case] message: &str) {
    let read = reader();
    let call = kwargs("cohere");
    let failure = match function {
        ReadFunction::FileRetrieve => read
            .run(
                ReadEntry::Sync,
                ReadOperation::Retrieve { file_id: "file-1" },
                &call,
                bare(),
            )
            .await
            .unwrap_err(),
        ReadFunction::FileList => read
            .run(
                ReadEntry::Sync,
                ReadOperation::List { purpose: None },
                &call,
                bare(),
            )
            .await
            .unwrap_err(),
    };
    assert!(matches!(
        &failure,
        FileReadError::Unsupported { provider, function: raised } if provider == "cohere" && *raised == function
    ));
    assert_eq!(failure.to_string(), message);
    assert_eq!(failure.status(), Some(400));
    assert_eq!(sent(&read), Vec::new());
}

#[rstest]
#[case::retrieve(ReadFunction::FileRetrieve, "file_retrieve", "CREATE_THREAD")]
#[case::list(ReadFunction::FileList, "file_list", "FILE_LIST")]
fn unsupported_file_read_placeholder(
    #[case] function: ReadFunction,
    #[case] name: &str,
    #[case] method: &str,
) {
    assert_eq!(function.name(), name);
    assert_eq!(function.request_method(), method);
    assert_eq!(UNSUPPORTED_BODY, "Unsupported provider");
    assert_eq!(UNSUPPORTED_MODEL, "n/a");
    assert_eq!(UNSUPPORTED_URL, "https://github.com/BerriAI/litellm");
}

#[tokio::test]
async fn unsupported_file_read_unknown_provider() {
    let read = reader();
    let call = kwargs("bogus");
    let retrieve = read
        .run(
            ReadEntry::Sync,
            ReadOperation::Retrieve { file_id: "file-1" },
            &call,
            bare(),
        )
        .await;
    let list = read
        .run(
            ReadEntry::Sync,
            ReadOperation::List { purpose: None },
            &call,
            bare(),
        )
        .await;
    assert!(
        matches!(&retrieve, Err(FileReadError::UnknownProvider { name, .. }) if name == "bogus")
    );
    assert!(matches!(list, Err(FileReadError::UnknownProvider { .. })));
    assert_eq!(list.unwrap_err().status(), None);
    assert_eq!(sent(&read), Vec::new());
}
