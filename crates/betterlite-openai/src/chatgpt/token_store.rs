use rust_decimal::{Decimal, prelude::ToPrimitive as _};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_config::sections::chatgpt_token_store::Settings;
use secrecy::{ExposeSecret as _, SecretString};

use self::codec::AuthRecord;
use self::files::{FileAccess, StoreEvent};
use self::member::Member;

pub mod codec;
pub mod files;
pub mod member;
mod tests;

pub trait Clock {
    fn now(&self) -> Decimal;
    fn sleep(&mut self, duration: Duration);
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("cannot create ChatGPT token directory")]
    Directory(#[source] io::Error),
}

pub struct ChatGptTokenStore<F, C> {
    pub files: F,
    pub clock: C,
    pub settings: Settings,
    pub auth_file: PathBuf,
}

impl<F: FileAccess, C: Clock> ChatGptTokenStore<F, C> {
    pub fn new(settings: Settings, mut files: F, clock: C) -> Result<Self, StoreError> {
        files
            .ensure_dir(Path::new(&settings.token_dir))
            .map_err(StoreError::Directory)?;
        let auth_file = Path::new(&settings.token_dir).join(&settings.auth_file);
        Ok(Self {
            files,
            clock,
            settings,
            auth_file,
        })
    }

    pub fn read(&mut self) -> Option<AuthRecord> {
        let bytes = self.files.read(&self.auth_file).ok()?;
        if let Ok(record) = codec::decode_auth_record(&bytes) {
            Some(record)
        } else {
            self.files.report(StoreEvent::InvalidRecord);
            None
        }
    }

    pub fn write(&mut self, record: &AuthRecord) {
        if self.files.write(&self.auth_file, record).is_err() {
            self.files.report(StoreEvent::WriteFailure);
        }
    }

    pub fn is_expired(&mut self, record: &mut AuthRecord, token: &SecretString) -> bool {
        let expiry = if let Some(Member::Known(value)) = &record.expires_at {
            Some(value.clone())
        } else {
            let derived = codec::expires_at(token);
            if let Some(value) = derived
                .as_ref()
                .filter(|value| value.as_f64().is_some_and(|seconds| seconds != 0.0_f64))
            {
                record.expires_at = Some(Member::Known(value.clone()));
                self.write(record);
            }
            derived
        };
        expiry.is_none_or(|value| {
            value.as_str().parse::<Decimal>().map_or_else(
                |_| {
                    value
                        .as_f64()
                        .zip(
                            self.clock
                                .now()
                                .checked_add(Decimal::from(60_i32))
                                .and_then(|boundary| boundary.to_f64()),
                        )
                        .is_none_or(|(seconds, boundary)| boundary >= seconds)
                },
                |seconds| {
                    seconds
                        .checked_sub(Decimal::from(60_i32))
                        .is_none_or(|boundary| self.clock.now() >= boundary)
                },
            )
        })
    }

    pub fn get_account_id(&mut self) -> Option<String> {
        let mut record = self.read()?;
        if let Some(cached) = record
            .account_id
            .as_ref()
            .and_then(Member::known)
            .filter(|value| !value.is_empty())
        {
            return Some(cached.clone());
        }
        let derived = codec::account_id(&record);
        if derived.is_some() {
            record.account_id = derived.clone().map(Member::Known);
            self.write(&record);
        }
        derived
    }

    #[must_use]
    pub fn cooldown_remaining(&self, record: Option<&AuthRecord>) -> Decimal {
        let requested = match record.and_then(|value| value.device_code_requested_at.as_ref()) {
            Some(Member::Known(value)) => value.as_str().parse::<Decimal>().ok(),
            Some(Member::Other(value)) => value
                .as_str()
                .and_then(|text| text.trim().parse::<Decimal>().ok()),
            None => None,
        };
        requested
            .and_then(|value| value.checked_add(Decimal::from(300_i32)))
            .and_then(|deadline| deadline.checked_sub(self.clock.now()))
            .unwrap_or(Decimal::ZERO)
            .max(Decimal::ZERO)
    }

    pub fn record_device_request(&mut self) {
        let mut record = self.read().unwrap_or_default();
        record.device_code_requested_at =
            self.clock.now().to_string().parse().ok().map(Member::Known);
        self.write(&record);
    }

    pub fn wait_for_access_token(&mut self, timeout: Duration) -> Option<SecretString> {
        let seconds = Decimal::from_f64_retain(timeout.as_secs_f64())?;
        let deadline = self.clock.now().checked_add(seconds)?;
        while self.clock.now() < deadline {
            if let Some(mut record) = self.read()
                && let Some(token) = record
                    .access_token
                    .as_ref()
                    .and_then(Member::known)
                    .cloned()
                    .filter(|value| !value.expose_secret().is_empty())
                && !self.is_expired(&mut record, &token)
            {
                return Some(token);
            }
            let remaining = deadline
                .checked_sub(self.clock.now())?
                .clamp(Decimal::ZERO, Decimal::from(5_i32));
            let sleep = Duration::try_from_secs_f64(remaining.to_f64()?).ok()?;
            if sleep.is_zero() {
                break;
            }
            self.clock.sleep(sleep);
        }
        None
    }
}
