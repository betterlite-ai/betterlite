use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::oidc_token_exchange::exchange::{TOKEN_EXCHANGE_PARAM, exchange_pkce_code};

use super::support::{LocalProvider, Reply, VALID_ID_TOKEN, base_request, client, text};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_max_level(Level::DEBUG)
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured log output is UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logged = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("log buffer poisoned"))?;
        logged.extend_from_slice(buf);
        drop(logged);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

#[rstest]
#[case::failed_status(
    400,
    r#"{"error":"invalid_grant","access_token":"LEAKED-ACCESS-TOKEN"}"#
)]
#[case::invalid_json(200, r#"{"access_token":"LEAKED-ACCESS-TOKEN""#)]
#[tokio::test]
async fn keeps_a_rejected_token_body_out_of_the_logs(#[case] status: u16, #[case] body: &str) {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::status(status, body));
    let (logs, _guard) = Logs::capture();
    let error = exchange_pkce_code(&client(), &base_request(&provider))
        .await
        .unwrap_err();
    assert_eq!(error.response().param, Some(TOKEN_EXCHANGE_PARAM));
    let captured = logs.text();
    assert!(!captured.contains("LEAKED-ACCESS-TOKEN"));
    assert!(captured.contains("the SSO token endpoint"));
}

#[tokio::test]
async fn keeps_a_rejected_userinfo_body_out_of_the_logs() {
    let provider = LocalProvider::start().await;
    provider.reply(
        "/token",
        Reply::json(&format!(
            r#"{{"access_token":"access-1","id_token":"{VALID_ID_TOKEN}"}}"#
        )),
    );
    provider.reply(
        "/userinfo",
        Reply::status(500, r#"{"refresh_token":"LEAKED-REFRESH-TOKEN"}"#),
    );
    let (logs, _guard) = Logs::capture();
    let claims = exchange_pkce_code(&client(), &base_request(&provider))
        .await
        .unwrap();
    assert_eq!(text(&claims, "sub"), Some("user-1"));
    let captured = logs.text();
    assert!(!captured.contains("LEAKED-REFRESH-TOKEN"));
    assert!(captured.contains("falling back to the id_token"));
}

#[tokio::test]
async fn keeps_issued_tokens_out_of_a_successful_exchange_log() {
    let provider = LocalProvider::start().await;
    provider.reply(
        "/token",
        Reply::json(
            r#"{"access_token":"ISSUED-ACCESS-TOKEN","id_token":"ISSUED-ID-TOKEN","refresh_token":"ISSUED-REFRESH-TOKEN"}"#,
        ),
    );
    provider.reply("/userinfo", Reply::json(r#"{"sub":"user-1"}"#));
    let (logs, _guard) = Logs::capture();
    exchange_pkce_code(&client(), &base_request(&provider))
        .await
        .unwrap();
    let captured = logs.text();
    assert!(captured.contains("the SSO token endpoint answered the code exchange"));
    assert!(!captured.contains("ISSUED-ACCESS-TOKEN"));
    assert!(!captured.contains("ISSUED-ID-TOKEN"));
    assert!(!captured.contains("ISSUED-REFRESH-TOKEN"));
}
