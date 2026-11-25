#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_secret_managers::secret_lookup::ports::{
    OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
};
use futures_util::future::{Either, select};
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::support::{fixture, model_row, sealed, secrets, text};

// The OIDC read tells the test it reached the blocking pool and waits there until the test releases it.
struct BlockingOidc {
    started: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

impl OidcHttp for BlockingOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        let notify = self
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // a dropped receiver means the test already failed, and the fake has nothing left to wait for
        if let Some(sender) = notify {
            sender.send(()).unwrap_or_default();
        }
        let release = self
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(waiting) = release {
            waiting.blocking_recv().unwrap_or_default();
        }
        Err(OidcHttpError::Request {
            message: "the test released the decryption".to_owned(),
        })
    }
}

// Dropping the rotation future cannot abort a running decryption, so the server's tracker has to hold shutdown until it ends.
#[tokio::test]
async fn shutdown_waits_for_a_blocked_decryption_after_cancellation() {
    let mut fixture = fixture();
    fixture.store.models = vec![model_row(
        "m1",
        vec![("api_key", text(&sealed("os.environ/oidc/google/audience")))],
    )];
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let mut lookup = secrets();
    lookup.oidc_http = Arc::new(BlockingOidc {
        started: Mutex::new(Some(started_tx)),
        release: Mutex::new(Some(release_rx)),
    });
    fixture.secrets = Arc::new(lookup);
    let tracker = fixture.tasks.clone();
    let mut rotation = fixture.rotation();
    let running = Box::pin(rotation.run(None));
    let pending = match select(running, started_rx).await {
        Either::Left((_finished, _other)) => {
            panic!("the rotation finished while the decryption was blocked")
        }
        Either::Right((started, pending)) => {
            started.expect("the OIDC read starts");
            pending
        }
    };
    drop(pending);
    tracker.close();
    let waiting = timeout(Duration::from_millis(200), tracker.wait()).await;
    assert!(
        waiting.is_err(),
        "shutdown waits for the blocked decryption"
    );
    release_tx.send(()).expect("the decryption is released");
    timeout(Duration::from_secs(5), tracker.wait())
        .await
        .expect("the tracked decryption ends after the release");
}
