use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::Instant;

use crate::cyberark::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};
use crate::cyberark::{Clock, CyberArkSecretManager};
use crate::secret_read_dispatch::protocols::SyncSecretReader as _;

struct ScriptedConjur {
    requests: Mutex<Vec<(HttpMethod, String)>>,
}

impl ScriptedConjur {
    fn answer(&self, request: &ConjurRequest<'_>) -> ConjurResponse {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request.method, request.url.clone()));
        let body = match request.method {
            HttpMethod::Post => "authenticated",
            HttpMethod::Get => "cyberark-value",
        };
        ConjurResponse {
            status: 200,
            body: SecretString::from(body),
        }
    }
}

impl ConjurTransport for Arc<ScriptedConjur> {
    type Error = IoError;

    fn send(
        &self,
        request: ConjurRequest<'_>,
    ) -> impl Future<Output = Result<ConjurResponse, IoError>> + Send {
        ready(Ok(self.answer(&request)))
    }

    fn send_blocking(&self, request: ConjurRequest<'_>) -> Result<ConjurResponse, IoError> {
        Ok(self.answer(&request))
    }
}

#[test]
fn cyberark_reader_reads_through_the_manager() {
    let transport = Arc::new(ScriptedConjur {
        requests: Mutex::new(Vec::new()),
    });
    let environment = BTreeMap::from([("CYBERARK_API_KEY".to_owned(), "key".to_owned())]);
    let clock: Clock = Arc::new(Instant::now);
    let manager =
        CyberArkSecretManager::new(&environment, Arc::clone(&transport), clock).expect("a manager");
    let secret = manager.sync_read_secret("db/password").expect("a secret");
    assert_eq!(secret.expose_secret(), "cyberark-value");
    let requests = transport
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].0, HttpMethod::Post);
    assert!(requests[0].1.ends_with("/authn/default/admin/authenticate"));
    assert_eq!(requests[1].0, HttpMethod::Get);
    assert!(
        requests[1]
            .1
            .ends_with("/secrets/default/variable/db%2Fpassword")
    );
    drop(requests);
}
