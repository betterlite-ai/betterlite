use std::io::Error as IoError;
use std::time::Duration;

use betterlite_config::sections::smtp_transport::Settings;
use lettre::Transport as _;
use lettre::address::Address as EnvelopeAddress;
use lettre::address::{AddressError, Envelope};
use lettre::error::Error as LettreError;
use lettre::transport::smtp::Error as SmtpError;
use lettre::transport::smtp::SmtpTransport;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
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
    Envelope(#[from] LettreError),
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

struct Mailbox<'text> {
    name: Option<&'text str>,
    address: &'text str,
}

// LiteLLM hands From and To to smtplib, whose quoteaddr sends "Name <addr>" as MAIL FROM:<addr>
fn split_mailbox(text: &str) -> Mailbox<'_> {
    let trimmed = text.trim();
    let (Some(open), Some(close)) = (trimmed.rfind('<'), trimmed.rfind('>')) else {
        return Mailbox {
            name: None,
            address: trimmed,
        };
    };
    if close <= open {
        return Mailbox {
            name: None,
            address: trimmed,
        };
    }
    let name = trimmed
        .get(..open)
        .unwrap_or_default()
        .trim()
        .trim_matches('"');
    let address = trimmed
        .get(open.saturating_add(1)..close)
        .unwrap_or_default()
        .trim();
    Mailbox {
        name: (!name.is_empty()).then_some(name),
        address,
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
        let sender = split_mailbox(&self.sender);
        let receiver = split_mailbox(&self.receiver);
        let sender_address: EnvelopeAddress = sender.address.parse()?;
        let receiver_address: EnvelopeAddress = receiver.address.parse()?;
        let envelope = Envelope::new(Some(sender_address), vec![receiver_address])?;
        let raw = self.message_bytes()?;
        let mut transport = SmtpTransport::builder_dangerous(self.host.clone())
            .port(self.port)
            .timeout(Some(self.timeout));
        transport = match self.security {
            ConnectionSecurity::ImplicitTls => {
                transport.tls(Tls::Wrapper(TlsParameters::new(self.host.clone())?))
            }
            ConnectionSecurity::StartTls => {
                transport.tls(Tls::Required(TlsParameters::new(self.host.clone())?))
            }
            ConnectionSecurity::PlainText => transport,
        };
        if let Some(credentials) = self.credentials {
            transport = transport.credentials(credentials);
        }
        transport.build().send_raw(&envelope, &raw)?;
        Ok(())
    }

    fn message_bytes(&self) -> Result<Vec<u8>, DeliveryError> {
        let sender = split_mailbox(&self.sender);
        let receiver = split_mailbox(&self.receiver);
        let mut message = MessageBuilder::new();
        message = match sender.name {
            Some(name) => message.from((name, sender.address)),
            None => message.from(sender.address),
        };
        message = match receiver.name {
            Some(name) => message.to((name, receiver.address)),
            None => message.to(receiver.address),
        };
        Ok(message
            .subject(self.subject.as_str())
            .html_body(self.html.as_str())
            .write_to_vec()?)
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
