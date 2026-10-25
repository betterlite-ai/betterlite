use email_address::{EmailAddress, Options};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::RawObject;
use super::attribute::DirectoryAttribute;

record! { #[serde(rename_all = "camelCase")] User {
    pub schemas: Vec<String>,
    pub id: Option<String>,
    pub external_id: Option<String>,
    pub meta: Option<RawObject>,
    pub user_name: Option<String>,
    pub name: Option<UserName>,
    pub display_name: Option<String>,
    #[serde(default)] pub active: Active,
    pub emails: Option<Vec<UserEmail>>,
    pub groups: Option<Vec<UserGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub entitlements: Option<Vec<DirectoryAttribute>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub roles: Option<Vec<DirectoryAttribute>>,
    #[serde(
        rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User",
        alias = "enterprise_user",
        skip_serializing_if = "Option::is_none"
    )]
    pub enterprise: Option<EnterpriseUser>,
}}
// a PUT that omits active must not reactivate a deactivated account, so the decoder keeps whether it was sent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Active {
    Sent(ScimBool),
    #[serde(skip_deserializing)]
    Defaulted(ScimBool),
}
impl Default for Active {
    fn default() -> Self {
        Self::Defaulted(ScimBool(true))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LaxBool")]
pub struct ScimBool(pub bool);
// Pydantic's lax mode reads these spellings as booleans, and directories send them
#[derive(Deserialize)]
#[serde(untagged)]
enum LaxBool {
    Flag(bool),
    Number(i64),
    Text(String),
}
#[derive(Debug, Error)]
#[error("the value is not a valid boolean")]
pub struct BoolError;
impl TryFrom<LaxBool> for ScimBool {
    type Error = BoolError;
    fn try_from(input: LaxBool) -> Result<Self, BoolError> {
        match input {
            LaxBool::Flag(flag) => Ok(Self(flag)),
            LaxBool::Number(0) => Ok(Self(false)),
            LaxBool::Number(1) => Ok(Self(true)),
            LaxBool::Text(text) => match text.to_ascii_lowercase().as_str() {
                "0" | "f" | "n" | "no" | "off" | "false" => Ok(Self(false)),
                "1" | "t" | "y" | "on" | "yes" | "true" => Ok(Self(true)),
                _ => Err(BoolError),
            },
            LaxBool::Number(_) => Err(BoolError),
        }
    }
}
record! { #[serde(rename_all = "camelCase")] UserName {
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    pub formatted: Option<String>,
    pub middle_name: Option<String>,
    pub honorific_prefix: Option<String>,
    pub honorific_suffix: Option<String>,
}}
record! { UserEmail {
    pub value: Email,
    #[serde(rename = "type")] pub kind: Option<String>,
    pub primary: Option<ScimBool>,
}}
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
record! { UserGroup {
    pub value: String,
    pub display: Option<String>,
    #[serde(rename = "type", default)] pub kind: GroupKind,
}}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupKind(pub Option<String>);
impl Default for GroupKind {
    fn default() -> Self {
        Self(Some(String::from("direct")))
    }
}
record! { #[serde(rename_all = "camelCase")] EnterpriseUser {
    pub employee_number: Option<String>,
    pub cost_center: Option<String>,
    pub organization: Option<String>,
    pub division: Option<String>,
    pub department: Option<String>,
    pub manager: Option<Manager>,
}}
record! { #[serde(rename_all = "camelCase")] Manager {
    pub value: Option<String>,
    pub display_name: Option<String>,
    #[serde(rename = "$ref", alias = "ref")] pub reference: Option<String>,
}}
