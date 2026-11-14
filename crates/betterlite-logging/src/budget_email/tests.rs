#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::branding::resolve_branding;
use super::event::{EntityType, WebhookEvent, WebhookEventType};
use super::sender::EmailSender;
use super::team_members::get_all_team_member_emails;
use super::{BudgetEmailError, send_email_alert_using_smtp, send_team_budget_alert};
use crate::alert_settings::{LITELLM_LOGO_URL, LITELLM_SUPPORT_CONTACT};
use crate::smtp_send::SendEmailError;

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
        self.failure
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }
}

fn recording_sender() -> RecordingSender {
    RecordingSender {
        emails: Mutex::new(Vec::new()),
        failure: None,
    }
}

fn failing_sender(error: SendEmailError) -> RecordingSender {
    RecordingSender {
        emails: Mutex::new(Vec::new()),
        failure: Some(error),
    }
}

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn budget_event() -> WebhookEvent {
    WebhookEvent {
        event: WebhookEventType::BudgetCrossed,
        event_message: "budget_crossed".to_owned(),
        event_group: EntityType::User,
        spend: Decimal::from(12),
        max_budget: Some(Decimal::from(100)),
        soft_budget: None,
        token: None,
        customer_id: None,
        user_id: Some("user-1".to_owned()),
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: Some("user@example.com".to_owned()),
        key_alias: None,
        projected_exceeded_date: None,
        projected_spend: None,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}

const USER_EMAIL_HTML: &str = "\n            <img src=\"https:&#x2f;&#x2f;litellm-listing.s3.amazonaws.com&#x2f;litellm_logo.png\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n\n            <p> Hi user-1, <br/>\n\n            Your LLM API usage this month has reached your account's <b> monthly budget of $100 </b> <br /> <br />\n\n            API requests will be rejected until either (a) you increase your monthly budget or (b) your monthly usage resets at the beginning of the next calendar month. <br /> <br />\n\n            If you have any questions, please send an email to support@berri.ai <br /> <br />\n\n            Best, <br />\n            The LiteLLM team <br />\n            ";

const TEAM_EMAIL_HTML: &str = "\n    <img src=\"https:&#x2f;&#x2f;litellm-listing.s3.amazonaws.com&#x2f;litellm_logo.png\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" /> <br/><br/><br/>\n\n    Budget Crossed for Team <b> Alpha </b> <br/> <br/>\n\n    Your Teams LLM API usage has crossed it's <b> budget of $100 </b>, current spend is <b>$12.5</b><br /> <br />\n\n    API requests will be rejected until either (a) you increase your budget or (b) your budget gets reset <br /> <br />\n\n    If you have any questions, please send an email to support@berri.ai <br /> <br />\n\n    Best, <br />\n    The LiteLLM team <br />\n    ";

#[rstest]
#[case::defaults(&[], LITELLM_LOGO_URL, LITELLM_SUPPORT_CONTACT)]
#[case::sender_logo_wins(
    &[("SMTP_SENDER_LOGO", "https://sender.example/logo.png"), ("EMAIL_LOGO_URL", "https://email.example/logo.png")],
    "https://sender.example/logo.png",
    LITELLM_SUPPORT_CONTACT
)]
#[case::email_logo_falls_back(
    &[("EMAIL_LOGO_URL", "https://email.example/logo.png")],
    "https://email.example/logo.png",
    LITELLM_SUPPORT_CONTACT
)]
#[case::empty_sender_logo_falls_back(
    &[("SMTP_SENDER_LOGO", ""), ("EMAIL_LOGO_URL", "https://email.example/logo.png")],
    "https://email.example/logo.png",
    LITELLM_SUPPORT_CONTACT
)]
#[case::custom_support_contact(
    &[("EMAIL_SUPPORT_CONTACT", "help@example.com")],
    LITELLM_LOGO_URL,
    "help@example.com"
)]
#[case::empty_support_contact_falls_back(
    &[("SMTP_SENDER_LOGO", "https://sender.example/logo.png"), ("EMAIL_SUPPORT_CONTACT", "")],
    "https://sender.example/logo.png",
    LITELLM_SUPPORT_CONTACT
)]
fn branding_resolves_the_litellm_defaults(
    #[case] values: &[(&str, &str)],
    #[case] logo_url: &str,
    #[case] support_contact: &str,
) {
    let branding = resolve_branding(&environment(values));
    assert_eq!(branding.logo_url, logo_url);
    assert_eq!(branding.support_contact, support_contact);
}

#[tokio::test]
async fn user_budget_alert_sends_the_litellm_template() {
    let (logs, guard) = capture_logs();
    let event = budget_event();
    let sender = recording_sender();
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].receiver.as_deref(), Some("user@example.com"));
    assert_eq!(delivered[0].subject, "LiteLLM: budget_crossed");
    assert_eq!(delivered[0].html, USER_EMAIL_HTML);
    assert!(!logs.contents().contains("no recipient"));
    drop(guard);
}

#[tokio::test]
async fn team_budget_alert_sends_the_litellm_template() {
    let (logs, guard) = capture_logs();
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.team_alias = Some("Alpha".to_owned());
    event.spend = "12.5".parse::<Decimal>().unwrap();
    let sender = recording_sender();
    let result = send_team_budget_alert(&event, &environment(&[]), None, &sender).await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].receiver.as_deref(), Some(""));
    assert_eq!(
        delivered[0].subject,
        "LiteLLM budget_crossed for Team Alpha"
    );
    assert_eq!(delivered[0].html, TEAM_EMAIL_HTML);
    let output = logs.contents();
    assert!(output.contains("Sending Team Budget Alert for team=Alpha"));
    assert!(output.contains("Sending team budget alert to "));
    assert!(!output.contains("no recipient"));
    drop(guard);
}

#[tokio::test]
async fn team_events_send_the_user_email_then_the_team_email() {
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.team_alias = Some("Alpha".to_owned());
    let sender = recording_sender();
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 2);
    assert_eq!(delivered[0].receiver.as_deref(), Some("user@example.com"));
    assert_eq!(delivered[0].subject, "LiteLLM: budget_crossed");
    assert!(delivered[0].html.contains("<p> Hi user-1, <br/>"));
    assert_eq!(delivered[1].receiver.as_deref(), Some(""));
    assert_eq!(
        delivered[1].subject,
        "LiteLLM budget_crossed for Team Alpha"
    );
    assert!(
        delivered[1]
            .html
            .contains("Budget Crossed for Team <b> Alpha </b>")
    );
}

#[tokio::test]
async fn other_events_send_the_default_body() {
    let mut event = budget_event();
    event.event = WebhookEventType::KeyCreated;
    event.event_message = "key_created".to_owned();
    let sender = recording_sender();
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].subject, "LiteLLM: key_created");
    assert_eq!(delivered[0].html, "Alert from LiteLLM Server");
}

#[tokio::test]
async fn customized_branding_reaches_the_team_email() {
    let values = environment(&[
        ("SMTP_SENDER_LOGO", "https://cdn.example/logo.png"),
        ("EMAIL_LOGO_URL", "https://ignored.example/logo.png"),
        ("EMAIL_SUPPORT_CONTACT", "help@example.com"),
    ]);
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.team_alias = Some("Alpha".to_owned());
    let sender = recording_sender();
    let result = send_team_budget_alert(&event, &values, None, &sender).await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert!(
        delivered[0]
            .html
            .contains("src=\"https:&#x2f;&#x2f;cdn.example&#x2f;logo.png\"")
    );
    assert!(
        delivered[0]
            .html
            .contains("send an email to help@example.com")
    );
}

#[tokio::test]
async fn budget_email_values_are_html_escaped() {
    let hostile = r#"<b>&"'"#;
    let escaped = "&lt;b&gt;&amp;&quot;&#x27;";
    let values = environment(&[("EMAIL_SUPPORT_CONTACT", hostile)]);
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.user_id = Some(hostile.to_owned());
    event.team_alias = Some(hostile.to_owned());
    let sender = recording_sender();
    let result =
        send_email_alert_using_smtp(&event, AlertType::BudgetAlerts, &values, None, &sender).await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 2);
    let user_html = &delivered[0].html;
    assert!(user_html.contains(&format!("<p> Hi {escaped}, <br/>")));
    assert!(user_html.contains(&format!("send an email to {escaped}")));
    assert!(!user_html.contains(hostile));
    let team_html = &delivered[1].html;
    assert!(team_html.contains(&format!("Budget Crossed for Team <b> {escaped} </b>")));
    assert!(team_html.contains(&format!("send an email to {escaped}")));
    assert!(!team_html.contains(hostile));
}

#[tokio::test]
async fn user_budget_alert_renders_absent_fields_as_none() {
    let mut event = budget_event();
    event.user_id = None;
    event.max_budget = None;
    let sender = recording_sender();
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    let html = &delivered[0].html;
    assert!(html.contains("<p> Hi None, <br/>"));
    assert!(html.contains("monthly budget of $None"));
}

#[tokio::test]
async fn team_budget_alert_renders_absent_fields_as_none() {
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.max_budget = None;
    let sender = recording_sender();
    let result = send_team_budget_alert(&event, &environment(&[]), None, &sender).await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered[0].subject, "LiteLLM budget_crossed for Team None");
    let html = &delivered[0].html;
    assert!(html.contains("Budget Crossed for Team <b> None </b>"));
    assert!(html.contains("budget of $None"));
    assert!(html.contains("current spend is <b>$12</b>"));
}

#[tokio::test]
async fn a_missing_user_email_is_still_attempted() {
    let mut event = budget_event();
    event.user_email = None;
    let sender = recording_sender();
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].receiver.is_none());
}

#[tokio::test]
async fn a_missing_user_email_is_logged_without_the_event_token() {
    let (logs, guard) = capture_logs();
    let mut event = budget_event();
    event.user_email = None;
    event.token = Some(SecretString::from("sk-secret-token"));
    let sender = failing_sender(SendEmailError::MissingReceiverEmail);
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(matches!(
        result,
        Err(BudgetEmailError::Send(SendEmailError::MissingReceiverEmail))
    ));
    let output = logs.contents();
    assert!(output.contains("Trying to send email alert to no recipient"));
    assert!(!output.contains("sk-secret-token"));
    drop(guard);
    assert_eq!(sender.sent().len(), 1);
}

#[tokio::test]
async fn a_send_error_is_returned() {
    let event = budget_event();
    let sender = failing_sender(SendEmailError::MissingSenderEmail);
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(matches!(
        result,
        Err(BudgetEmailError::Send(SendEmailError::MissingSenderEmail))
    ));
    assert_eq!(sender.sent().len(), 1);
}

#[tokio::test]
async fn a_team_event_without_a_database_reports_it_after_the_user_email() {
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.team_id = Some("team-1".to_owned());
    event.team_alias = Some("Alpha".to_owned());
    let sender = recording_sender();
    let result = send_email_alert_using_smtp(
        &event,
        AlertType::BudgetAlerts,
        &environment(&[]),
        None,
        &sender,
    )
    .await;
    assert!(matches!(result, Err(BudgetEmailError::NoDatabase)));
    assert_eq!(sender.sent().len(), 1);
}

#[tokio::test]
async fn team_member_emails_returns_empty_without_a_team_id() {
    let emails = get_all_team_member_emails(None, None).await.unwrap();
    assert!(emails.is_empty());
}

#[tokio::test]
async fn team_member_emails_requires_a_database() {
    let result = get_all_team_member_emails(None, Some("team-1")).await;
    assert!(matches!(result, Err(BudgetEmailError::NoDatabase)));
}

#[derive(Parser)]
struct PostgresEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = PostgresEnvironment::parse_from(["budget-email-tests"])
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

#[tokio::test]
async fn team_member_emails_returns_every_member_email() {
    let (logs, guard) = capture_logs();
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models, members_with_roles) VALUES
            ('team-1', 'Alpha', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[], '[{"user_id":"u1","role":"user"},{"user_id":"u2","role":"admin"}]'::JSONB);
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
            ('u1', 'one@example.com'),
            ('u2', 'two@example.com');
        "#,
    )
    .await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let mut emails = get_all_team_member_emails(Some(&store), Some("team-1"))
        .await
        .unwrap();
    emails.sort_unstable();
    assert_eq!(emails, vec!["one@example.com", "two@example.com"]);
    let output = logs.contents();
    assert!(output.contains("Getting all team members for team_id=team-1"));
    drop(guard);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn team_member_emails_skips_blank_ids_and_missing_emails() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, admins, members, models, members_with_roles) VALUES
            ('team-1', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[], '[{"user_id":"","role":"user"},{"user_id":"u2","role":"user"},{"user_email":"ignored@example.com","role":"user"}]'::JSONB);
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
            ('', 'blank@example.com'),
            ('u2', NULL),
            ('u3', 'three@example.com');
        "#,
    )
    .await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let emails = get_all_team_member_emails(Some(&store), Some("team-1"))
        .await
        .unwrap();
    assert!(emails.is_empty());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn team_member_emails_returns_empty_for_an_unknown_team() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models) VALUES
            ('other', 'Beta', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[]);
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('u1', 'one@example.com');
        "#,
    )
    .await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let emails = get_all_team_member_emails(Some(&store), Some("team-1"))
        .await
        .unwrap();
    assert!(emails.is_empty());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn team_member_emails_returns_empty_for_an_empty_roster() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models) VALUES
            ('team-1', 'Alpha', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[]);
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ('u1', 'one@example.com');
        "#,
    )
    .await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let emails = get_all_team_member_emails(Some(&store), Some("team-1"))
        .await
        .unwrap();
    assert!(emails.is_empty());
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn team_member_emails_reports_a_broken_team_row() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, admins, members, models, members_with_roles) VALUES
            ('team-1', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[], '"not a roster"'::JSONB);
        "#,
    )
    .await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let result = get_all_team_member_emails(Some(&store), Some("team-1")).await;
    assert!(matches!(result, Err(BudgetEmailError::Team { .. })));
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn team_budget_alert_sends_to_all_member_emails() {
    let (fixture, pool) = database().await;
    seed(
        &pool,
        r#"
        INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias, admins, members, models, members_with_roles) VALUES
            ('team-1', 'Alpha', '{}'::TEXT[], '{}'::TEXT[], '{}'::TEXT[], '[{"user_id":"u1","role":"user"},{"user_id":"u2","role":"admin"}]'::JSONB);
        INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES
            ('u1', 'one@example.com'),
            ('u2', 'two@example.com');
        "#,
    )
    .await;
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    let mut event = budget_event();
    event.event_group = EntityType::Team;
    event.team_id = Some("team-1".to_owned());
    event.team_alias = Some("Alpha".to_owned());
    let sender = recording_sender();
    let result = send_team_budget_alert(&event, &environment(&[]), Some(&store), &sender).await;
    assert!(!result.unwrap());
    let delivered = sender.sent();
    assert_eq!(delivered.len(), 1);
    let receiver = delivered[0].receiver.as_deref().unwrap_or_default();
    let mut recipients: Vec<&str> = receiver.split(',').collect();
    recipients.sort_unstable();
    assert_eq!(recipients, vec!["one@example.com", "two@example.com"]);
    assert!(
        delivered[0]
            .html
            .contains("Budget Crossed for Team <b> Alpha </b>")
    );
    pool.close();
    fixture.close_async().await.unwrap();
}
