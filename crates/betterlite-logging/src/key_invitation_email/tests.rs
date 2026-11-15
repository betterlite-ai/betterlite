#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use minijinja::HtmlEscape;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::KeyInvitationEmailer;
use super::link::{InvitationCreator, InvitationId, construct_user_invitation_link};
use super::templates::{
    KeyCreatedEmail, UserInvitedEmail, render_key_created_email, render_user_invited_email,
};
use crate::alert_dispatch::destination::AlertDestination;
use crate::alert_settings::{LITELLM_LOGO_URL, LITELLM_SUPPORT_CONTACT};
use crate::budget_email::event::WebhookEventType;
use crate::budget_email::sender::EmailSender;
use crate::smtp_send::SendEmailError;

mod fixtures;

use fixtures::{budget_email, invited_email, invited_event, key_created_event};

const EMAIL_ONLY: &[AlertDestination] = &[AlertDestination::Email];
const SLACK_ONLY: &[AlertDestination] = &[AlertDestination::Slack];

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for LogBuffer {
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

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SentEmail {
    receiver: Option<String>,
    subject: String,
    html: String,
}

struct RecordingSender {
    emails: Mutex<Vec<SentEmail>>,
    failure: Option<SendEmailError>,
    reject_missing_receiver: bool,
}

impl RecordingSender {
    fn sent(&self) -> Vec<SentEmail> {
        self.emails.lock().unwrap().clone()
    }
}

#[async_trait]
impl EmailSender for RecordingSender {
    async fn send(
        &self,
        receiver: Option<&str>,
        subject: &str,
        html: &str,
    ) -> Result<(), SendEmailError> {
        self.emails.lock().unwrap().push(SentEmail {
            receiver: receiver.map(str::to_owned),
            subject: subject.to_owned(),
            html: html.to_owned(),
        });
        if self.reject_missing_receiver && receiver.is_none() {
            return Err(SendEmailError::MissingReceiverEmail);
        }
        self.failure
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }
}

fn recording_sender() -> RecordingSender {
    RecordingSender {
        emails: Mutex::new(Vec::new()),
        failure: None,
        reject_missing_receiver: false,
    }
}

fn strict_sender() -> RecordingSender {
    RecordingSender {
        emails: Mutex::new(Vec::new()),
        failure: None,
        reject_missing_receiver: true,
    }
}

fn failing_sender(error: SendEmailError) -> RecordingSender {
    RecordingSender {
        emails: Mutex::new(Vec::new()),
        failure: Some(error),
        reject_missing_receiver: false,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the invitation could not be created")]
struct CreatorFailure;

struct FakeCreator {
    calls: Mutex<Vec<String>>,
    invitation_id: String,
    failing: bool,
}

impl FakeCreator {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl InvitationCreator for FakeCreator {
    type Error = CreatorFailure;

    async fn create_for_user(&self, user_id: &str) -> Result<InvitationId, CreatorFailure> {
        self.calls.lock().unwrap().push(user_id.to_owned());
        if self.failing {
            return Err(CreatorFailure);
        }
        Ok(InvitationId(self.invitation_id.clone()))
    }
}

fn creating_creator() -> FakeCreator {
    FakeCreator {
        calls: Mutex::new(Vec::new()),
        invitation_id: "inv-created".to_owned(),
        failing: false,
    }
}

fn failing_creator() -> FakeCreator {
    FakeCreator {
        calls: Mutex::new(Vec::new()),
        invitation_id: String::new(),
        failing: true,
    }
}

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn emailer<'deps>(
    alerting: Option<&'deps [AlertDestination]>,
    environment: &'deps BTreeMap<String, String>,
    database: Option<&'deps PostgresStore>,
    sender: &'deps RecordingSender,
    invitation_creator: &'deps FakeCreator,
) -> KeyInvitationEmailer<'deps, FakeCreator> {
    KeyInvitationEmailer {
        alerting,
        environment,
        database,
        sender,
        invitation_creator,
    }
}

#[test]
fn render_key_created_email_matches_the_litellm_template() {
    let html = render_key_created_email(&budget_email(Some(10.0_f64)));
    assert_eq!(html, fixtures::KEY_CREATED_HTML);
    assert_eq!(html.matches("sk-test-token").count(), 2);
    assert!(html.contains("base_url={base_url}"));
    assert!(!html.contains("proxy.example"));
    assert!(!html.contains("{{"));
    assert!(!html.contains("}}"));
    let braced = render_key_created_email(&KeyCreatedEmail {
        key_token: Some("sk-{email_logo_url}"),
        ..budget_email(Some(10.0_f64))
    });
    assert!(braced.contains("Key: <pre>sk-{email_logo_url}</pre> <br>"));
    assert!(braced.contains("api_key=\"sk-{email_logo_url}\""));
}

#[test]
fn render_key_created_email_prints_none_for_missing_values() {
    let email = KeyCreatedEmail {
        recipient_email: None,
        key_budget: None,
        key_token: None,
        ..budget_email(None)
    };
    let html = render_key_created_email(&email);
    assert_eq!(html, fixtures::KEY_CREATED_MISSING_HTML);
    assert!(html.contains("<p> Hi None, <br/>"));
    assert!(html.contains("loaded with $None per month"));
    assert!(html.contains("Key: <pre>None</pre> <br>"));
    assert!(html.contains("api_key=\"None\""));
}

#[test]
fn render_user_invited_email_matches_the_litellm_template() {
    let email = invited_email("https://proxy.example/ui/onboarding?invitation_id=inv-1");
    let html = render_user_invited_email(&email);
    assert_eq!(html, fixtures::USER_INVITED_HTML);
    assert!(html.contains("<p> Hi bob@example.com, <br/>"));
    assert!(
        html.contains("You were invited to use OpenAI Proxy API for team Alpha  <br /> <br />")
    );
    assert!(html.contains(
        "<a href=\"https:&#x2f;&#x2f;proxy.example&#x2f;ui&#x2f;onboarding?invitation_id=inv-1\" style=\"display: inline-block;"
    ));
}

#[test]
fn render_escapes_html_in_substituted_values() {
    let key = render_key_created_email(&KeyCreatedEmail {
        email_logo_url: "a\"b",
        recipient_email: Some("<i>&'"),
        key_token: Some("<b>"),
        email_support_contact: "<s>",
        ..budget_email(Some(10.0_f64))
    });
    assert!(key.contains("<img src=\"a&quot;b\""));
    assert!(key.contains("Hi &lt;i&gt;&amp;&#x27;, <br/>"));
    assert!(key.contains("Key: <pre>&lt;b&gt;</pre> <br>"));
    assert!(key.contains("api_key=\"&lt;b&gt;\""));
    assert!(key.contains("send an email to &lt;s&gt; <br />"));
    let invited = render_user_invited_email(&UserInvitedEmail {
        team_name: "<a href=x>team</a>",
        ..invited_email("x\"y")
    });
    assert!(invited.contains("for team &lt;a href=x&gt;team&lt;&#x2f;a&gt;  <br />"));
    assert!(invited.contains("<a href=\"x&quot;y\" style="));
}

#[rstest]
#[case::whole(10.0_f64, "10.0")]
#[case::fraction(0.5_f64, "0.5")]
#[case::cents(100.25_f64, "100.25")]
#[case::small(1e-7_f64, "1e-07")]
#[case::large(1e300_f64, "1e+300")]
#[case::negative_zero(-0.0_f64, "-0.0")]
fn render_float_budgets_print_like_python(#[case] budget: f64, #[case] expected: &str) {
    let html = render_key_created_email(&budget_email(Some(budget)));
    assert!(html.contains(&format!("loaded with ${expected} per month")));
}

#[rstest]
#[case::nan(f64::NAN, "nan")]
#[case::infinity(f64::INFINITY, "inf")]
#[case::negative_infinity(f64::NEG_INFINITY, "-inf")]
fn render_non_finite_budgets_print_like_python(#[case] budget: f64, #[case] expected: &str) {
    let html = render_key_created_email(&budget_email(Some(budget)));
    assert!(html.contains(&format!("loaded with ${expected} per month")));
}

#[derive(Parser)]
struct PostgresEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = PostgresEnvironment::parse_from(["key-invitation-email-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client
        .batch_execute(include_str!(
            "../../../../tools/postgres-fixture/fixtures/baseline.sql"
        ))
        .await
        .unwrap();
    drop(client);
    (fixture, pool)
}

async fn seed(pool: &Pool, sql: &str) {
    let client = pool.get().await.unwrap();
    client.batch_execute(sql).await.unwrap();
    drop(client);
}

fn database_store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

async fn close(pool: &Pool, fixture: PostgresFixture) {
    pool.close();
    fixture.close_async().await.unwrap();
}

const TEAM_ALPHA_SQL: &str = r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models) VALUES ('team-1', 'Alpha', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[]);"#;

#[tokio::test]
async fn send_returns_false_without_alerting() {
    let (logs, guard) = capture_logs();
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(None, &env, None, &sender, &creator);
    let event = key_created_event();
    assert!(!mailer.send_key_created_or_user_invited_email(&event).await);
    assert!(sender.sent().is_empty());
    assert!(logs.contents().contains("'email' not in the alerting list"));
    drop(guard);
}

#[tokio::test]
async fn send_returns_false_when_alerting_lacks_email() {
    let (logs, guard) = capture_logs();
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(SLACK_ONLY), &env, None, &sender, &creator);
    let event = key_created_event();
    assert!(!mailer.send_key_created_or_user_invited_email(&event).await);
    assert!(sender.sent().is_empty());
    assert!(
        logs.contents()
            .contains("'email' not in the alerting list: Some([Slack])")
    );
    drop(guard);
}

#[tokio::test]
async fn send_key_created_uses_the_default_branding() {
    let (logs, guard) = capture_logs();
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let event = key_created_event();
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].receiver.as_deref(), Some("alice@example.com"));
    assert_eq!(delivered[0].subject, "LiteLLM: key_created");
    let html = &delivered[0].html;
    assert!(html.contains(&format!("src=\"{}\"", HtmlEscape(LITELLM_LOGO_URL))));
    assert!(html.contains(&format!("send an email to {LITELLM_SUPPORT_CONTACT}")));
    assert!(html.contains("loaded with $10.0 per month"));
    assert!(html.contains("Key: <pre>sk-test-token</pre> <br>"));
    assert!(html.contains("api_key=\"sk-test-token\""));
    assert!(!logs.contents().contains("sk-test-token"));
    drop(guard);
}

#[tokio::test]
async fn send_uses_customized_branding_without_a_license_check() {
    let env = environment(&[
        ("SMTP_SENDER_LOGO", "https://cdn.example/logo.png"),
        ("EMAIL_LOGO_URL", "https://ignored.example/logo.png"),
        ("EMAIL_SUPPORT_CONTACT", "help@example.com"),
    ]);
    let sender = recording_sender();
    let creator = creating_creator();
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let event = key_created_event();
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    let html = &delivered[0].html;
    assert!(html.contains("src=\"https:&#x2f;&#x2f;cdn.example&#x2f;logo.png\""));
    assert!(!html.contains("ignored.example"));
    assert!(html.contains("send an email to help@example.com"));
    assert!(!html.contains(&HtmlEscape(LITELLM_LOGO_URL).to_string()));
    assert!(!html.contains(LITELLM_SUPPORT_CONTACT));
}

#[tokio::test]
async fn send_empty_branding_values_fall_back_to_the_defaults() {
    let env = environment(&[
        ("SMTP_SENDER_LOGO", ""),
        ("EMAIL_LOGO_URL", ""),
        ("EMAIL_SUPPORT_CONTACT", ""),
    ]);
    let sender = recording_sender();
    let creator = creating_creator();
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let event = key_created_event();
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    let html = &delivered[0].html;
    assert!(html.contains(&format!("src=\"{}\"", HtmlEscape(LITELLM_LOGO_URL))));
    assert!(html.contains(&format!("send an email to {LITELLM_SUPPORT_CONTACT}")));
}

#[tokio::test]
async fn send_key_created_without_budget_or_token_prints_none() {
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let mut event = key_created_event();
    event.max_budget = None;
    event.token = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    let html = &delivered[0].html;
    assert!(html.contains("loaded with $None per month"));
    assert!(html.contains("api_key=\"None\""));
}

#[tokio::test]
async fn send_reads_a_missing_recipient_from_the_user_row() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('user-1', 'looked-up@example.com');"#,
    )
    .await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let mut event = key_created_event();
    event.user_email = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].receiver.as_deref(),
        Some("looked-up@example.com")
    );
    assert!(
        delivered[0]
            .html
            .contains("<p> Hi looked-up@example.com, <br/>")
    );
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_without_a_recipient_logs_and_fails_inside_the_sender() {
    let (logs, guard) = capture_logs();
    let sender = strict_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let mut event = key_created_event();
    event.user_email = None;
    event.user_id = None;
    assert!(!mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].receiver.is_none());
    let output = logs.contents();
    assert!(
        output.contains("Trying to send email alert to no recipient, event=key_created user_id=")
    );
    assert!(output.contains("Error sending email alert"));
    assert!(!output.contains("xyzzy"));
    assert!(!output.contains("sk-test-token"));
    drop(guard);
}

#[tokio::test]
async fn send_internal_user_created_uses_the_team_alias() {
    let (fixture, pool) = database().await;
    seed(&pool, TEAM_ALPHA_SQL).await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let event = invited_event();
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].receiver.as_deref(), Some("bob@example.com"));
    assert_eq!(delivered[0].subject, "LiteLLM: internal_user_created");
    let html = &delivered[0].html;
    assert!(
        html.contains("You were invited to use OpenAI Proxy API for team Alpha  <br /> <br />")
    );
    assert!(html.contains(
        "href=\"http:&#x2f;&#x2f;0.0.0.0:4000&#x2f;ui&#x2f;onboarding?invitation_id=inv-created\""
    ));
    assert_eq!(creator.calls(), vec!["user-1"]);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_internal_user_created_escapes_a_team_alias_with_markup() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models) VALUES ('team-1', '<b>Evil</b>', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[]);"#,
    )
    .await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    assert!(
        mailer
            .send_key_created_or_user_invited_email(&invited_event())
            .await
    );
    let delivered = sender.sent();
    assert!(
        delivered[0]
            .html
            .contains("for team &lt;b&gt;Evil&lt;&#x2f;b&gt;  <br /> <br />")
    );
    assert!(!delivered[0].html.contains("<b>Evil</b>"));
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_internal_user_created_uses_a_dash_for_an_empty_or_null_alias() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models) VALUES
            ('team-1', '', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[]),
            ('team-2', NULL, '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[]);
        "#,
    )
    .await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let mut event = invited_event();
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    event.team_id = Some("team-2".to_owned());
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 2);
    assert!(
        delivered[0]
            .html
            .contains("You were invited to use OpenAI Proxy API for team -  <br /> <br />")
    );
    assert!(
        delivered[1]
            .html
            .contains("You were invited to use OpenAI Proxy API for team -  <br /> <br />")
    );
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_internal_user_created_uses_the_default_team_without_a_team() {
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let mut event = invited_event();
    event.team_id = None;
    event.user_id = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    let html = &delivered[0].html;
    assert!(html.contains("You were invited to use OpenAI Proxy API for team Default Team"));
    assert!(html.contains("href=\"http:&#x2f;&#x2f;0.0.0.0:4000\""));
    assert!(creator.calls().is_empty());
}

#[tokio::test]
async fn send_internal_user_created_uses_the_default_team_for_a_missing_row() {
    let (fixture, pool) = database().await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let event = invited_event();
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert!(
        delivered[0]
            .html
            .contains("You were invited to use OpenAI Proxy API for team Default Team")
    );
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_internal_user_created_links_an_existing_invitation() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('user-1', 'bob@example.com');
        INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES
            ('inv-existing', 'user-1', '2024-02-01', '2024-01-01', 'user-1', '2024-01-01', 'user-1');
        "#,
    )
    .await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let mut event = invited_event();
    event.team_id = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert!(delivered[0].html.contains(
        "href=\"http:&#x2f;&#x2f;0.0.0.0:4000&#x2f;ui&#x2f;onboarding?invitation_id=inv-existing\""
    ));
    assert!(creator.calls().is_empty());
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_internal_user_created_keeps_the_base_url_when_linking_fails() {
    let (logs, guard) = capture_logs();
    let (fixture, pool) = database().await;
    seed(&pool, r#"DROP TABLE "LiteLLM_InvitationLink";"#).await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let mut event = invited_event();
    event.team_id = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert!(
        delivered[0]
            .html
            .contains("href=\"http:&#x2f;&#x2f;0.0.0.0:4000\"")
    );
    assert!(creator.calls().is_empty());
    assert!(
        logs.contents()
            .contains("Error creating invitation link for user_id user-1")
    );
    drop(guard);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_unknown_event_sends_the_default_body() {
    let (logs, guard) = capture_logs();
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let mut event = key_created_event();
    event.event = WebhookEventType::BudgetCrossed;
    event.event_message = "Budget alert".to_owned();
    event.token = Some(SecretString::from("sk-secret-token"));
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].subject, "LiteLLM: Budget alert");
    assert_eq!(delivered[0].html, "Alert from LiteLLM Server");
    let output = logs.contents();
    assert!(
        output
            .contains("Trying to send email alert on unknown webhook event, event=budget_crossed")
    );
    assert!(!output.contains("sk-secret-token"));
    drop(guard);
}

#[tokio::test]
async fn send_sender_error_is_logged_and_returns_false() {
    let (logs, guard) = capture_logs();
    let sender = failing_sender(SendEmailError::MissingSenderEmail);
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let event = key_created_event();
    assert!(!mailer.send_key_created_or_user_invited_email(&event).await);
    assert_eq!(sender.sent().len(), 1);
    assert!(
        logs.contents()
            .contains("Error sending email alert: the email could not be sent")
    );
    drop(guard);
}

#[tokio::test]
async fn send_user_lookup_failure_is_logged_and_returns_false() {
    let (logs, guard) = capture_logs();
    let (fixture, pool) = database().await;
    seed(&pool, r#"DROP TABLE "LiteLLM_UserTable" CASCADE;"#).await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let mut event = key_created_event();
    event.user_email = None;
    assert!(!mailer.send_key_created_or_user_invited_email(&event).await);
    assert!(sender.sent().is_empty());
    assert!(logs.contents().contains("Error sending email alert"));
    drop(guard);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_team_lookup_failure_is_logged_and_returns_false() {
    let (logs, guard) = capture_logs();
    let (fixture, pool) = database().await;
    seed(&pool, r#"DROP TABLE "LiteLLM_TeamTable" CASCADE;"#).await;
    let store = database_store(&pool);
    let sender = recording_sender();
    let creator = creating_creator();
    let env = environment(&[]);
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let event = invited_event();
    assert!(!mailer.send_key_created_or_user_invited_email(&event).await);
    assert!(sender.sent().is_empty());
    assert!(logs.contents().contains("Error sending email alert"));
    drop(guard);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn send_empty_proxy_base_url_falls_back_to_the_default() {
    let env = environment(&[("PROXY_BASE_URL", "")]);
    let sender = recording_sender();
    let creator = creating_creator();
    let mailer = emailer(Some(EMAIL_ONLY), &env, None, &sender, &creator);
    let mut event = invited_event();
    event.team_id = None;
    event.user_id = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert!(
        delivered[0]
            .html
            .contains("href=\"http:&#x2f;&#x2f;0.0.0.0:4000\"")
    );
}

#[tokio::test]
async fn send_custom_proxy_base_url_trims_trailing_slashes() {
    let (fixture, pool) = database().await;
    let store = database_store(&pool);
    let env = environment(&[("PROXY_BASE_URL", "https://proxy.example/")]);
    let sender = recording_sender();
    let creator = creating_creator();
    let mailer = emailer(Some(EMAIL_ONLY), &env, Some(&store), &sender, &creator);
    let mut event = invited_event();
    event.team_id = None;
    assert!(mailer.send_key_created_or_user_invited_email(&event).await);
    let delivered = sender.sent();
    assert!(delivered[0].html.contains(
        "href=\"https:&#x2f;&#x2f;proxy.example&#x2f;ui&#x2f;onboarding?invitation_id=inv-created\""
    ));
    close(&pool, fixture).await;
}

#[tokio::test]
async fn link_without_a_user_id_returns_the_base_url() {
    let (fixture, pool) = database().await;
    let store = database_store(&pool);
    let creator = creating_creator();
    let link =
        construct_user_invitation_link(None, "https://proxy.example/", Some(&store), &creator)
            .await;
    assert_eq!(link, "https://proxy.example/");
    assert!(creator.calls().is_empty());
    close(&pool, fixture).await;
}

#[tokio::test]
async fn link_without_a_database_returns_the_base_url() {
    let creator = creating_creator();
    let link =
        construct_user_invitation_link(Some("user-1"), "https://proxy.example", None, &creator)
            .await;
    assert_eq!(link, "https://proxy.example");
    assert!(creator.calls().is_empty());
}

#[tokio::test]
async fn link_uses_the_newest_invitation() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
            ('user-1', 'one@example.com'),
            ('user-2', 'two@example.com');
        INSERT INTO "LiteLLM_InvitationLink" (id, user_id, expires_at, created_at, created_by, updated_at, updated_by) VALUES
            ('old', 'user-1', '2024-01-08', '2024-01-01', 'user-1', '2024-01-01', 'user-1'),
            ('other', 'user-2', '2024-12-08', '2024-12-01', 'user-2', '2024-12-01', 'user-2'),
            ('new', 'user-1', '2024-06-08', '2024-06-01', 'user-1', '2024-06-01', 'user-1');
        "#,
    )
    .await;
    let store = database_store(&pool);
    let creator = creating_creator();
    let link = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example/",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(
        link,
        "https://proxy.example/ui/onboarding?invitation_id=new"
    );
    assert!(creator.calls().is_empty());
    close(&pool, fixture).await;
}

#[tokio::test]
async fn link_creates_an_invitation_only_when_none_exists() {
    let (fixture, pool) = database().await;
    let store = database_store(&pool);
    let creator = creating_creator();
    let link = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(
        link,
        "https://proxy.example/ui/onboarding?invitation_id=inv-created"
    );
    assert_eq!(creator.calls(), vec!["user-1"]);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn link_returns_the_base_url_when_creation_fails() {
    let (logs, guard) = capture_logs();
    let (fixture, pool) = database().await;
    let store = database_store(&pool);
    let creator = failing_creator();
    let link = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(link, "https://proxy.example");
    assert_eq!(creator.calls(), vec!["user-1"]);
    let output = logs.contents();
    assert!(output.contains("Error creating invitation link for user_id user-1"));
    assert!(output.contains("the invitation could not be created"));
    drop(guard);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn link_returns_the_base_url_when_the_lookup_fails() {
    let (logs, guard) = capture_logs();
    let (fixture, pool) = database().await;
    seed(&pool, r#"DROP TABLE "LiteLLM_InvitationLink";"#).await;
    let store = database_store(&pool);
    let creator = creating_creator();
    let link = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(link, "https://proxy.example");
    assert!(creator.calls().is_empty());
    assert!(
        logs.contents()
            .contains("Error creating invitation link for user_id user-1")
    );
    drop(guard);
    close(&pool, fixture).await;
}

#[tokio::test]
async fn link_trims_every_trailing_slash() {
    let (fixture, pool) = database().await;
    let store = database_store(&pool);
    let creator = creating_creator();
    let expected = "https://proxy.example/ui/onboarding?invitation_id=inv-created";
    let plain = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(plain, expected);
    let with_slash = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example/",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(with_slash, expected);
    let with_slashes = construct_user_invitation_link(
        Some("user-1"),
        "https://proxy.example///",
        Some(&store),
        &creator,
    )
    .await;
    assert_eq!(with_slashes, expected);
    assert_eq!(creator.calls(), vec!["user-1", "user-1", "user-1"]);
    close(&pool, fixture).await;
}
