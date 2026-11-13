use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::auth_config::oauth::TokenExchangeProfile;
use secrecy::SecretString;

use super::{JWT_BEARER_GRANT, ON_BEHALF_OF, TOKEN_EXCHANGE_GRANT};

pub(super) type ExchangeForm = BTreeMap<String, SecretString>;

fn insert_scope(form: &mut ExchangeForm, scopes: &[String]) {
    if !scopes.is_empty() {
        form.insert("scope".to_owned(), SecretString::from(scopes.join(" ")));
    }
}

pub(super) fn build_exchange_form(
    profile: TokenExchangeProfile,
    subject_token: &str,
    subject_token_type: &str,
    audience: Option<&str>,
    scopes: &[String],
) -> ExchangeForm {
    let mut form = ExchangeForm::new();
    match profile {
        TokenExchangeProfile::Rfc8693 => {
            form.insert(
                "grant_type".to_owned(),
                SecretString::from(TOKEN_EXCHANGE_GRANT),
            );
            form.insert(
                "subject_token".to_owned(),
                SecretString::from(subject_token),
            );
            form.insert(
                "subject_token_type".to_owned(),
                SecretString::from(subject_token_type),
            );
            if let Some(configured) = audience.filter(|value| !value.is_empty()) {
                form.insert("audience".to_owned(), SecretString::from(configured));
            }
            insert_scope(&mut form, scopes);
        }
        TokenExchangeProfile::EntraObo => {
            form.insert(
                "grant_type".to_owned(),
                SecretString::from(JWT_BEARER_GRANT),
            );
            form.insert("assertion".to_owned(), SecretString::from(subject_token));
            insert_scope(&mut form, scopes);
            form.insert(
                "requested_token_use".to_owned(),
                SecretString::from(ON_BEHALF_OF),
            );
        }
    }
    form
}
