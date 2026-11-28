use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use super::fixture::{
    ACCESS_KEY, Factory, FixtureClient, ManagedReader, Reply, SECRET_KEY, SESSION_TOKEN, Source,
    credentials, environment, headers, managed_secrets, managed_secrets_reader, now, object, raw,
    runtime,
};
use crate::bedrock_embedding_io::credentials::{
    CredentialRequest, LoadedCredentials, ManagedRegionWorker, load_credentials,
};
use crate::bedrock_embedding_io::error::EmbeddingIoError;
use crate::bedrock_embedding_io::signed_get::sign_get_request;
use crate::bedrock_embedding_io::transport::{EmbeddingCall, EmbeddingRequest, post_embedding};
use betterlite_core::api::vector_store_http_write::service::SuppliedClient;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use secrecy::SecretString;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::timeout;

const URL: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/amazon.titan-embed-text-v1/invoke";

struct PendingReader {
    gate: Arc<(Mutex<bool>, Condvar)>,
    started: Arc<Notify>,
    read_on_runtime: Arc<AtomicBool>,
}

impl SyncSecretReader for PendingReader {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        self.read_on_runtime
            .store(Handle::try_current().is_ok(), Ordering::SeqCst);
        self.started.notify_one();
        let (gate_lock, gate_condition) = &*self.gate;
        let mut gate_state = gate_lock.lock().unwrap();
        while !*gate_state {
            gate_state = gate_condition.wait(gate_state).unwrap();
        }
        drop(gate_state);
        (secret_name == "AWS_REGION_NAME").then(|| SecretString::from("eu-north-1"))
    }
}

struct GateRelease(Arc<(Mutex<bool>, Condvar)>);

impl GateRelease {
    fn open(&self) {
        let (gate_lock, gate_condition) = &*self.0;
        *gate_lock.lock().unwrap() = true;
        gate_condition.notify_all();
    }
}

impl Drop for GateRelease {
    fn drop(&mut self) {
        self.open();
    }
}

fn single_worker_runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("the fixture runtime builds")
}

fn pending_lookup(
    gate: &Arc<(Mutex<bool>, Condvar)>,
    started: &Arc<Notify>,
    read_on_runtime: &Arc<AtomicBool>,
) -> SecretLookup {
    managed_secrets_reader(Arc::new(PendingReader {
        gate: Arc::clone(gate),
        started: Arc::clone(started),
        read_on_runtime: Arc::clone(read_on_runtime),
    }))
}

fn lookup_tasks(
    secrets: Arc<SecretLookup>,
    region_worker: Arc<ManagedRegionWorker>,
) -> JoinSet<Result<LoadedCredentials, EmbeddingIoError>> {
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let mut parameters = raw(&[]);
        let variables = environment(&[]);
        let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
        load_credentials(
            CredentialRequest {
                optional_params: &mut parameters,
                environment: &variables,
                bearer_token: None,
            },
            &secrets,
            &region_worker,
            source.resolve(),
        )
        .await
    });
    tasks
}

fn timeout_while_managed_read_waits(
    secrets: Arc<SecretLookup>,
    started: &Notify,
    region_worker: Arc<ManagedRegionWorker>,
) -> (
    Runtime,
    JoinSet<Result<LoadedCredentials, EmbeddingIoError>>,
) {
    let runtime = single_worker_runtime();
    let tasks = runtime.block_on(async {
        let mut tasks = lookup_tasks(secrets, region_worker);
        timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("the managed reader starts");
        timeout(Duration::from_millis(50), tasks.join_next())
            .await
            .expect_err("the lookup waits for the managed reader");
        tasks
    });
    (runtime, tasks)
}

#[test]
fn bedrock_embedding_io_managed_region_secret_is_read_off_runtime() {
    let reader = Arc::new(ManagedReader::new("eu-north-1"));
    let secrets = managed_secrets(Arc::clone(&reader));
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    let mut parameters = raw(&[]);
    let variables = environment(&[("AWS_REGION_NAME", "us-east-1")]);
    let region_worker = ManagedRegionWorker::new().expect("the region worker starts");
    let loaded = runtime()
        .block_on(load_credentials(
            CredentialRequest {
                optional_params: &mut parameters,
                environment: &variables,
                bearer_token: None,
            },
            &secrets,
            &region_worker,
            source.resolve(),
        ))
        .expect("the managed region secret loads");
    assert_eq!(loaded.region, "eu-north-1");
    assert_eq!(source.calls()[0].1, "eu-north-1");
    assert_eq!(
        reader.reads(),
        [
            ("AWS_REGION_NAME".to_owned(), false),
            ("AWS_REGION".to_owned(), false),
        ]
    );
}

#[test]
fn bedrock_embedding_io_managed_lookup_allows_timeout_while_pending() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let started = Arc::new(Notify::new());
    let read_on_runtime = Arc::new(AtomicBool::new(true));
    let secrets = Arc::new(pending_lookup(&gate, &started, &read_on_runtime));
    let region_worker = Arc::new(ManagedRegionWorker::new().expect("the region worker starts"));
    let gate_release = GateRelease(Arc::clone(&gate));
    let (runtime, mut tasks) = timeout_while_managed_read_waits(
        Arc::clone(&secrets),
        &started,
        Arc::clone(&region_worker),
    );
    gate_release.open();
    let loaded = runtime.block_on(async {
        tasks
            .join_next()
            .await
            .expect("the managed lookup task completes")
            .expect("the managed lookup task does not panic")
            .expect("the worker drains cancelled lookups and handles the next one")
    });
    assert_eq!(loaded.region, "eu-north-1");
    assert!(!read_on_runtime.load(Ordering::SeqCst));
}

#[test]
fn bedrock_embedding_io_managed_worker_drains_before_drop() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let started = Arc::new(Notify::new());
    let read_on_runtime = Arc::new(AtomicBool::new(true));
    let secrets = Arc::new(pending_lookup(&gate, &started, &read_on_runtime));
    let worker = Arc::new(ManagedRegionWorker::new().expect("the region worker starts"));
    let gate_release = GateRelease(Arc::clone(&gate));
    let runtime = single_worker_runtime();
    runtime.block_on(async {
        let mut lookups = lookup_tasks(Arc::clone(&secrets), Arc::clone(&worker));
        timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("the managed reader starts");
        lookups.abort_all();
        lookups
            .join_next()
            .await
            .expect("the task is collected")
            .expect_err("the aborted lookup ends cancelled");
    });
    let owned_worker =
        Arc::try_unwrap(worker).unwrap_or_else(|_| panic!("the lookup released its worker"));
    runtime.block_on(async {
        let mut owners = JoinSet::new();
        owners.spawn_blocking(move || drop(owned_worker));
        timeout(Duration::from_millis(50), owners.join_next())
            .await
            .expect_err("dropping the worker waits for the managed reader");
        gate_release.open();
        assert!(matches!(
            timeout(Duration::from_secs(1), owners.join_next()).await,
            Ok(Some(Ok(())))
        ));
    });
    assert!(!read_on_runtime.load(Ordering::SeqCst));
}

#[test]
fn bedrock_embedding_io_secret_safe_failure() {
    let (sent, error) = signed_failure_request();
    assert_signed_headers_are_present(&sent);
    assert_fixture_secrets_are_absent(&sent, &error);
}

fn signed_failure_request() -> (EmbeddingRequest, EmbeddingIoError) {
    let signed = sign_get_request(
        &credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN)),
        URL,
        &headers(&[("Content-Type", "application/json")]),
        "us-west-2",
        now(),
    )
    .expect("the fixture request signs");
    let data = object(r#"{"inputText":"hello"}"#);
    let factory = Factory::new(Reply::status(401, &[], "denied"));
    let call = EmbeddingCall {
        asynchronous: false,
        url: URL,
        headers: &signed.headers,
        data: &data,
        timeout: None,
    };
    let error = runtime()
        .block_on(post_embedding(
            &call,
            None::<SuppliedClient<'_, FixtureClient>>,
            &factory,
        ))
        .expect_err("the fixture status fails");
    let recorded = factory.requests();
    assert_eq!(recorded.len(), 1);
    (recorded[0].clone(), error)
}

fn assert_signed_headers_are_present(sent: &EmbeddingRequest) {
    assert!(sent.headers.iter().any(|(name, value)| {
        name.as_str() == "X-Amz-Security-Token" && value.as_str() == SESSION_TOKEN
    }));
    assert!(
        sent.headers
            .iter()
            .any(|(name, value)| name.as_str() == "Authorization" && value.contains(ACCESS_KEY)),
        "the signed request really carries the credential"
    );
    assert!(
        format!("{sent:?}").contains("Authorization"),
        "the request Debug still names the signed headers"
    );
}

fn assert_fixture_secrets_are_absent(sent: &EmbeddingRequest, error: &EmbeddingIoError) {
    let rendered = [
        format!("{sent:?}"),
        format!(
            "{:?}",
            credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN))
        ),
        format!("{error:?}"),
        error.to_string(),
    ];
    assert!(
        rendered.iter().all(|text| {
            [ACCESS_KEY, SECRET_KEY, SESSION_TOKEN]
                .iter()
                .all(|value| !text.contains(value))
        }),
        "a rendering names a fixture secret"
    );
}
