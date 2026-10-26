use email_address::{EmailAddress, Options};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Email(pub String);
#[derive(Debug, Error)]
pub enum EmailError {
    #[error("the email address is not valid: {0}")]
    Syntax(#[source] email_address::Error),
    #[error("Pydantic's EmailStr refuses this address")]
    Refused,
}
impl TryFrom<String> for Email {
    type Error = EmailError;
    fn try_from(raw: String) -> Result<Self, EmailError> {
        let options = Options::default()
            .with_required_tld()
            .without_domain_literal();
        let address =
            EmailAddress::parse_with_options(raw.trim(), options).map_err(EmailError::Syntax)?;
        let domain = address.domain().to_lowercase();
        let tld = domain.rsplit('.').next().unwrap_or_default();
        let local = address.local_part();
        // email-validator refuses special-use names and top-level domains that end in a digit
        let special_use = matches!(
            tld,
            "arpa" | "invalid" | "local" | "localhost" | "onion" | "test"
        );
        if raw.contains(['\r', '\n'])
            || local.starts_with('"')
            || special_use
            || !tld.ends_with(char::is_alphabetic)
        {
            return Err(EmailError::Refused);
        }
        let mailbox = local.to_ascii_lowercase();
        let stored = match mailbox.as_str() {
            // RFC 2142 role mailboxes, which email-validator lowercases
            "info" | "marketing" | "sales" | "support" | "abuse" | "noc" | "security"
            | "postmaster" | "hostmaster" | "usenet" | "news" | "webmaster" | "www" | "uucp"
            | "ftp" => mailbox.as_str(),
            _ => local,
        };
        Ok(Self(format!("{stored}@{domain}")))
    }
}
