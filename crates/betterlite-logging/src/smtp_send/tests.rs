#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::smtp_transport::Settings;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{ConnectionSecurity, SendEmailError, connection_security, send_email};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServerMode {
    Plain,
    OffersStartTls,
    OffersStartTlsAndAuth,
    TlsFirst,
}

struct FakeSmtp {
    port: u16,
    transcript: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

impl FakeSmtp {
    fn lines(&self) -> Vec<String> {
        self.transcript
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn serve(mode: ServerMode) -> FakeSmtp {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let transcript = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = JoinSet::new();
    tasks.spawn(handle(listener, mode, Arc::clone(&transcript)));
    FakeSmtp {
        port,
        transcript,
        _tasks: tasks,
    }
}

async fn handle(listener: TcpListener, mode: ServerMode, transcript: Arc<Mutex<Vec<String>>>) {
    let Ok((stream, _)) = listener.accept().await else {
        return;
    };
    if mode == ServerMode::TlsFirst {
        record_first_byte(stream, &transcript).await;
        return;
    }
    let (reader, mut writer) = stream.into_split();
    writer.write_all(b"220 test ESMTP\r\n").await.unwrap();
    let mut lines = BufReader::new(reader).lines();
    let mut in_data = false;
    while let Ok(Some(line)) = lines.next_line().await {
        transcript
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line.clone());
        if !answer(&line, mode, &mut in_data, &mut writer).await {
            return;
        }
    }
}

async fn record_first_byte(stream: TcpStream, transcript: &Arc<Mutex<Vec<String>>>) {
    let mut first = [0_u8; 1];
    let Ok(read) = stream.peek(&mut first).await else {
        return;
    };
    if read == 0 {
        return;
    }
    transcript
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(format!("first-byte:{}", first[0]));
}

async fn answer_data(line: &str, in_data: &mut bool, writer: &mut OwnedWriteHalf) {
    if line == "." {
        *in_data = false;
        writer.write_all(b"250 OK\r\n").await.unwrap();
    }
}

async fn answer(
    line: &str,
    mode: ServerMode,
    in_data: &mut bool,
    writer: &mut OwnedWriteHalf,
) -> bool {
    if *in_data {
        answer_data(line, in_data, writer).await;
        return true;
    }
    match line {
        "DATA" => {
            *in_data = true;
            writer.write_all(b"354 End data\r\n").await.unwrap();
        }
        "QUIT" => {
            writer.write_all(b"221 Bye\r\n").await.unwrap();
            return false;
        }
        _ if line.starts_with("EHLO") => write_features(writer, mode).await,
        _ if line.starts_with("STARTTLS") => {
            writer
                .write_all(b"220 Ready to start TLS\r\n")
                .await
                .unwrap();
        }
        _ if line.starts_with("AUTH") => {
            writer
                .write_all(b"235 2.7.0 Authentication successful\r\n")
                .await
                .unwrap();
        }
        _ if line.starts_with("MAIL FROM") || line.starts_with("RCPT TO") => {
            writer.write_all(b"250 OK\r\n").await.unwrap();
        }
        _ => writer.write_all(b"500 Unknown command\r\n").await.unwrap(),
    }
    true
}

async fn write_features(writer: &mut OwnedWriteHalf, mode: ServerMode) {
    writer.write_all(b"250-test\r\n").await.unwrap();
    if matches!(
        mode,
        ServerMode::OffersStartTls | ServerMode::OffersStartTlsAndAuth
    ) {
        writer.write_all(b"250-STARTTLS\r\n").await.unwrap();
    }
    if mode == ServerMode::OffersStartTlsAndAuth {
        writer.write_all(b"250-AUTH PLAIN LOGIN\r\n").await.unwrap();
    }
    writer.write_all(b"250 OK\r\n").await.unwrap();
}

fn test_settings(port: u16) -> Settings {
    Settings {
        host: Some("127.0.0.1".to_owned()),
        port,
        username: None,
        password: None,
        sender_email: Some("sender@example.com".to_owned()),
        use_ssl: false,
        tls: false,
        timeout: Duration::from_secs(2),
    }
}

async fn deliver(settings: &Settings) -> Result<(), SendEmailError> {
    send_email(
        settings,
        Some("receiver@example.com"),
        Some("Hello"),
        Some("<p>hi</p>"),
    )
    .await
}

#[test]
fn port_465_picks_implicit_tls() {
    let mut settings = test_settings(465);
    settings.use_ssl = false;
    settings.tls = true;
    assert_eq!(
        connection_security(&settings),
        ConnectionSecurity::ImplicitTls
    );
}

#[test]
fn use_ssl_picks_implicit_tls() {
    let mut settings = test_settings(587);
    settings.use_ssl = true;
    settings.tls = true;
    assert_eq!(
        connection_security(&settings),
        ConnectionSecurity::ImplicitTls
    );
}

#[test]
fn tls_on_picks_starttls() {
    let mut settings = test_settings(587);
    settings.use_ssl = false;
    settings.tls = true;
    assert_eq!(connection_security(&settings), ConnectionSecurity::StartTls);
}

#[test]
fn tls_off_picks_plain_text() {
    let mut settings = test_settings(587);
    settings.use_ssl = false;
    settings.tls = false;
    assert_eq!(
        connection_security(&settings),
        ConnectionSecurity::PlainText
    );
}

#[tokio::test]
async fn sends_the_message_over_plain_smtp() {
    let server = serve(ServerMode::Plain).await;
    let outcome = deliver(&test_settings(server.port)).await;
    assert_eq!(outcome, Ok(()));
    let lines = server.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("MAIL FROM:<sender@example.com>"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("RCPT TO:<receiver@example.com>"))
    );
    assert!(lines.iter().any(|line| line == "DATA"));
}

#[tokio::test]
async fn plain_smtp_skips_starttls_even_when_the_server_offers_it() {
    let server = serve(ServerMode::OffersStartTls).await;
    let outcome = deliver(&test_settings(server.port)).await;
    assert_eq!(outcome, Ok(()));
    assert!(!server.lines().iter().any(|line| line == "STARTTLS"));
}

#[tokio::test]
async fn use_ssl_starts_a_tls_handshake_immediately() {
    let server = serve(ServerMode::TlsFirst).await;
    let mut settings = test_settings(server.port);
    settings.use_ssl = true;
    let outcome = deliver(&settings).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(
        server.lines().first().map(String::as_str),
        Some("first-byte:22")
    );
}

#[rstest]
#[case::both(Some("mailer"), Some("hunter2"), true)]
#[case::username_only(Some("mailer"), None, false)]
#[case::password_only(None, Some("hunter2"), false)]
#[case::neither(None, None, false)]
#[tokio::test]
async fn login_runs_only_with_both_credentials(
    #[case] username: Option<&str>,
    #[case] password: Option<&str>,
    #[case] expected: bool,
) {
    let server = serve(ServerMode::OffersStartTlsAndAuth).await;
    let mut settings = test_settings(server.port);
    settings.username = username.map(str::to_owned);
    settings.password = password.map(SecretString::from);
    let outcome = deliver(&settings).await;
    assert_eq!(outcome, Ok(()));
    assert_eq!(
        server.lines().iter().any(|line| line.starts_with("AUTH")),
        expected
    );
}

#[tokio::test]
async fn the_sender_is_checked_before_the_host() {
    let mut settings = test_settings(587);
    settings.sender_email = None;
    settings.host = None;
    assert_eq!(
        deliver(&settings).await,
        Err(SendEmailError::MissingSenderEmail)
    );
}

#[tokio::test]
async fn a_missing_receiver_is_reported() {
    let settings = test_settings(587);
    let outcome = send_email(&settings, None, Some("Hello"), Some("<p>hi</p>")).await;
    assert_eq!(outcome, Err(SendEmailError::MissingReceiverEmail));
}

#[tokio::test]
async fn a_missing_subject_is_reported() {
    let settings = test_settings(587);
    let outcome = send_email(
        &settings,
        Some("receiver@example.com"),
        None,
        Some("<p>hi</p>"),
    )
    .await;
    assert_eq!(outcome, Err(SendEmailError::MissingSubject));
}

#[tokio::test]
async fn a_missing_html_body_is_reported() {
    let settings = test_settings(587);
    let outcome = send_email(&settings, Some("receiver@example.com"), Some("Hello"), None).await;
    assert_eq!(outcome, Err(SendEmailError::MissingHtml));
}

#[tokio::test]
async fn a_missing_host_is_reported_after_the_message_fields() {
    let mut settings = test_settings(587);
    settings.host = None;
    assert_eq!(deliver(&settings).await, Err(SendEmailError::MissingHost));
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn a_send_failure_is_logged_and_not_returned() {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(buffer.clone())
        .finish();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let (outcome, transcript) = with_default(subscriber, || {
        runtime.block_on(async {
            let server = serve(ServerMode::Plain).await;
            let mut settings = test_settings(server.port);
            settings.tls = true;
            let outcome = deliver(&settings).await;
            (outcome, server.lines())
        })
    });
    assert_eq!(outcome, Ok(()));
    assert!(!transcript.iter().any(|line| line == "STARTTLS"));
    assert!(
        buffer
            .contents()
            .contains("An error occurred while sending the email:")
    );
}
