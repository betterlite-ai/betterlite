use std::io::Error as IoError;
use std::time::Duration;

use betterlite_config::sections::smtp_transport::Settings;
use lettre::address::Address as EnvelopeAddress;
use lettre::address::AddressError;
use lettre::transport::smtp::Error as SmtpError;
use lettre::transport::smtp::authentication::{Credentials, DEFAULT_MECHANISMS};
use lettre::transport::smtp::client::{SmtpConnection, TlsParameters};
use lettre::transport::smtp::commands::{Data, Mail, Rcpt};
use lettre::transport::smtp::extension::ClientId;
use mail_builder::MessageBuilder;
use secrecy::ExposeSecret as _;
use tokio::task::JoinSet;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendEmailError {
    #[error("Trying to use SMTP, but SMTP_SENDER_EMAIL is not set")]
    MissingSenderEmail,
    #[error("No receiver email provided for SMTP email.")]
    MissingReceiverEmail,
    #[error("No subject provided for SMTP email.")]
    MissingSubject,
    #[error("No HTML body provided for SMTP email.")]
    MissingHtml,
    #[error("Trying to use SMTP, but SMTP_HOST is not set")]
    MissingHost,
}

#[derive(Debug, thiserror::Error)]
enum DeliveryError {
    #[error(transparent)]
    Address(#[from] AddressError),
    #[error(transparent)]
    Message(#[from] IoError),
    #[error(transparent)]
    Smtp(#[from] SmtpError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionSecurity {
    ImplicitTls,
    StartTls,
    PlainText,
}

pub(crate) const fn connection_security(settings: &Settings) -> ConnectionSecurity {
    if settings.use_ssl || settings.port == 465 {
        ConnectionSecurity::ImplicitTls
    } else if settings.tls {
        ConnectionSecurity::StartTls
    } else {
        ConnectionSecurity::PlainText
    }
}

struct Delivery {
    host: String,
    port: u16,
    security: ConnectionSecurity,
    credentials: Option<Credentials>,
    timeout: Duration,
    sender: String,
    receiver: String,
    subject: String,
    html: String,
}

impl Delivery {
    fn new(
        settings: &Settings,
        receiver_email: Option<&str>,
        subject: Option<&str>,
        html: Option<&str>,
    ) -> Result<Self, SendEmailError> {
        let sender = settings
            .sender_email
            .clone()
            .ok_or(SendEmailError::MissingSenderEmail)?;
        let receiver = receiver_email.ok_or(SendEmailError::MissingReceiverEmail)?;
        let subject_line = subject.ok_or(SendEmailError::MissingSubject)?;
        let body = html.ok_or(SendEmailError::MissingHtml)?;
        let host = settings.host.clone().ok_or(SendEmailError::MissingHost)?;
        let credentials = settings
            .username
            .as_ref()
            .zip(settings.password.as_ref())
            .map(|(username, password)| {
                Credentials::new(username.clone(), password.expose_secret().to_owned())
            });
        Ok(Self {
            host,
            port: settings.port,
            security: connection_security(settings),
            credentials,
            timeout: settings.timeout,
            sender,
            receiver: receiver.to_owned(),
            subject: subject_line.to_owned(),
            html: body.to_owned(),
        })
    }

    fn send(self) -> Result<(), DeliveryError> {
        let sender: EnvelopeAddress = self.sender.parse()?;
        let receiver: EnvelopeAddress = self.receiver.parse()?;
        let mut connection = connect(&self)?;
        if let Some(credentials) = &self.credentials {
            connection.auth(DEFAULT_MECHANISMS, credentials)?;
        }
        let raw = MessageBuilder::new()
            .from(self.sender)
            .to(self.receiver)
            .subject(self.subject)
            .html_body(self.html)
            .write_to_vec()?;
        connection.command(Mail::new(Some(sender), Vec::new()))?;
        connection.command(Rcpt::new(receiver, Vec::new()))?;
        connection.command(Data)?;
        connection.message(&raw)?;
        connection.abort();
        Ok(())
    }
}

fn connect(delivery: &Delivery) -> Result<SmtpConnection, DeliveryError> {
    let client = ClientId::default();
    let endpoint = (delivery.host.as_str(), delivery.port);
    match delivery.security {
        ConnectionSecurity::ImplicitTls => {
            let tls = TlsParameters::new(delivery.host.clone())?;
            Ok(SmtpConnection::connect(
                endpoint,
                Some(delivery.timeout),
                &client,
                Some(&tls),
                None,
            )?)
        }
        ConnectionSecurity::StartTls => {
            let tls = TlsParameters::new(delivery.host.clone())?;
            let mut connection =
                SmtpConnection::connect(endpoint, Some(delivery.timeout), &client, None, None)?;
            connection.starttls(&tls, &client)?;
            Ok(connection)
        }
        ConnectionSecurity::PlainText => Ok(SmtpConnection::connect(
            endpoint,
            Some(delivery.timeout),
            &client,
            None,
            None,
        )?),
    }
}

pub async fn send_email(
    settings: &Settings,
    receiver_email: Option<&str>,
    subject: Option<&str>,
    html: Option<&str>,
) -> Result<(), SendEmailError> {
    let delivery = Delivery::new(settings, receiver_email, subject, html)?;
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || delivery.send());
    match tasks.join_next().await {
        Some(Ok(Ok(()))) => Ok(()),
        Some(Ok(Err(error))) => {
            tracing::error!("An error occurred while sending the email:{error}");
            Ok(())
        }
        Some(Err(error)) => {
            tracing::error!("An error occurred while sending the email:{error}");
            Ok(())
        }
        None => {
            tracing::error!(
                "An error occurred while sending the email:the delivery task did not run"
            );
            Ok(())
        }
    }
}
