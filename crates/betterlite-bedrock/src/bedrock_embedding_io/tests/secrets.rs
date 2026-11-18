use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use super::fixture::{
    ACCESS_KEY, Factory, FixtureClient, ManagedReader, Reply, SECRET_KEY, SESSION_TOKEN, Source,
    credentials, environment, headers, managed_secrets, managed_secrets_reader, now, object, raw,
    runtime,
};
use crate::bedrock_embedding_io::credentials::{CredentialRequest, load_credentials};
use crate::bedrock_embedding_io::signed_get::sign_get_request;
use crate::bedrock_embedding_io::transport::{
    EmbeddingCall, EmbeddingRequest, SuppliedClient, post_embedding,
};
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use secrecy::SecretString;

const URL: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/amazon.titan-embed-text-v1/invoke";

struct PendingReader {
    gate: Arc<(Mutex<bool>, Condvar)>,
    progress: Arc<AtomicBool>,
    progressed_while_pending: Arc<AtomicBool>,
}

impl SyncSecretReader for PendingReader {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        let (released, ready) = &*self.gate;
        let mut released = released.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
        self.progressed_while_pending
            .store(self.progress.load(Ordering::SeqCst), Ordering::SeqCst);
        (secret_name == "AWS_REGION_NAME").then(|| SecretString::from("eu-north-1"))
    }
}

#[test]
fn bedrock_embedding_io_managed_region_secret_is_read_off_runtime() {
    let reader = Arc::new(ManagedReader::new("eu-north-1"));
    let secrets = managed_secrets(Arc::clone(&reader));
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    let mut parameters = raw(&[]);
    let variables = environment(&[("AWS_REGION_NAME", "us-east-1")]);
    let loaded = runtime()
        .block_on(load_credentials(
            CredentialRequest {
                optional_params: &mut parameters,
                environment: &variables,
                bearer_token: None,
            },
            &secrets,
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
fn bedrock_embedding_io_pending_managed_lookup_yields_tokio_worker() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let progress = Arc::new(AtomicBool::new(false));
    let progressed_while_pending = Arc::new(AtomicBool::new(false));
    let secrets = managed_secrets_reader(Arc::new(PendingReader {
        gate: Arc::clone(&gate),
        progress: Arc::clone(&progress),
        progressed_while_pending: Arc::clone(&progressed_while_pending),
    }));
    let (progress_tx, progress_rx) = std::sync::mpsc::channel();
    let release_gate = Arc::clone(&gate);
    let release = std::thread::spawn(move || {
        let _ = progress_rx.recv_timeout(Duration::from_secs(2));
        let (released, ready) = &*release_gate;
        *released.lock().unwrap() = true;
        ready.notify_all();
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("the fixture runtime builds");
    let mut parameters = raw(&[]);
    let variables = environment(&[]);
    let source = Source::new(credentials(ACCESS_KEY, SECRET_KEY, None));
    runtime
        .block_on(async {
            let progress = Arc::clone(&progress);
            tokio::spawn(async move {
                progress.store(true, Ordering::SeqCst);
                progress_tx.send(()).expect("the release waiter is alive");
            });
            crate::bedrock_embedding_io::credentials::load_credentials(
                crate::bedrock_embedding_io::credentials::CredentialRequest {
                    optional_params: &mut parameters,
                    environment: &variables,
                    bearer_token: None,
                },
                &secrets,
                source.resolve(),
            )
            .await
            .expect("the managed region lookup completes")
        })
        .region;
    release.join().expect("the release worker completes");
    assert!(progressed_while_pending.load(Ordering::SeqCst));
}

#[test]
fn bedrock_embedding_io_secret_safe_failure() {
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
    let sent: &EmbeddingRequest = &recorded[0];
    assert!(sent.headers.iter().any(|(name, value)| {
        name.as_str() == "X-Amz-Security-Token" && value.as_str() == SESSION_TOKEN
    }));
    assert!(
        sent.headers
            .iter()
            .any(|(name, value)| name.as_str() == "Authorization" && value.contains(ACCESS_KEY)),
        "the signed request really carries the credential"
    );
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
    assert!(
        rendered[0].contains("Authorization"),
        "the request Debug still names the signed headers"
    );
}
