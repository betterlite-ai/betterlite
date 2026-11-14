use betterlite_config::loader::EnvironmentLookup;

use crate::alert_settings::{LITELLM_LOGO_URL, LITELLM_SUPPORT_CONTACT};

pub struct Branding {
    pub logo_url: String,
    pub support_contact: String,
}

#[must_use]
pub fn resolve_branding(environment: &dyn EnvironmentLookup) -> Branding {
    let logo_url = environment
        .lookup("SMTP_SENDER_LOGO")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            environment
                .lookup("EMAIL_LOGO_URL")
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| LITELLM_LOGO_URL.to_owned());
    let support_contact = environment
        .lookup("EMAIL_SUPPORT_CONTACT")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| LITELLM_SUPPORT_CONTACT.to_owned());
    Branding {
        logo_url,
        support_contact,
    }
}
