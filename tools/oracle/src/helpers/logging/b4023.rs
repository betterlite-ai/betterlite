use betterlite_logging::key_invitation_email::templates::{
    KeyCreatedEmail, UserInvitedEmail, render_key_created_email, render_user_invited_email,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let html = match input {
            Input::KeyCreated {
                email_logo_url,
                recipient_email,
                key_budget,
                key_token,
                base_url,
                email_support_contact,
            } => render_key_created_email(&KeyCreatedEmail {
                email_logo_url,
                recipient_email: recipient_email.as_deref(),
                key_budget: *key_budget,
                key_token: key_token.as_deref(),
                base_url,
                email_support_contact,
            }),
            Input::UserInvited {
                email_logo_url,
                recipient_email,
                team_name,
                base_url,
                email_support_contact,
            } => render_user_invited_email(&UserInvitedEmail {
                email_logo_url,
                recipient_email: recipient_email.as_deref(),
                team_name,
                base_url,
                email_support_contact,
            }),
        };
        Ok(json!({ "html": html }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "template", rename_all = "snake_case")]
pub(crate) enum Input {
    KeyCreated {
        email_logo_url: String,
        #[serde(default)]
        recipient_email: Option<String>,
        #[serde(default)]
        // ast-grep-ignore: no-float-money
        key_budget: Option<f64>,
        #[serde(default)]
        key_token: Option<String>,
        base_url: String,
        email_support_contact: String,
    },
    UserInvited {
        email_logo_url: String,
        #[serde(default)]
        recipient_email: Option<String>,
        team_name: String,
        base_url: String,
        email_support_contact: String,
    },
}
