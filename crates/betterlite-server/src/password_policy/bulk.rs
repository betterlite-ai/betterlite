use std::collections::HashSet;

use betterlite_config::sections::password_policy::Settings;
use futures_util::StreamExt as _;
use futures_util::stream;
use secrecy::{ExposeSecret as _, SecretString};

use super::breach::BreachChecker;
use super::error::PasswordError;
use super::strength;

const MAX_CONCURRENT_LOOKUPS: usize = 64;

#[derive(Debug)]
pub struct Verdict {
    pub password: SecretString,
    pub error: Option<PasswordError>,
}

// Each unique password is judged once, in first-seen order; only those that pass the strength rules reach the breach lookup.
pub async fn validate_passwords_bulk(
    passwords: &[SecretString],
    settings: &Settings,
    checker: &BreachChecker,
) -> Vec<Verdict> {
    let mut seen = HashSet::new();
    let unique: Vec<&SecretString> = passwords
        .iter()
        .filter(|password| seen.insert(password.expose_secret()))
        .collect();
    let mut verdicts: Vec<Verdict> = unique
        .iter()
        .map(|password| Verdict {
            password: (*password).clone(),
            error: strength::validate(password, settings).err(),
        })
        .collect();
    let screened: Vec<&mut Verdict> = verdicts
        .iter_mut()
        .filter(|verdict| verdict.error.is_none())
        .collect();
    let breached: Vec<bool> = stream::iter(screened.iter())
        .map(|verdict| checker.is_breached(&verdict.password))
        .buffered(MAX_CONCURRENT_LOOKUPS)
        .collect()
        .await;
    for (verdict, is_breached) in screened.into_iter().zip(breached) {
        if is_breached {
            verdict.error = Some(PasswordError::Breached);
        }
    }
    verdicts
}
