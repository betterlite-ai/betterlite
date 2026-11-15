use secrecy::{ExposeSecret as _, SecretString};

pub const MASTER_KEY_SETTING: &str = "master_key";
pub const OVERRIDE_SETTING: &str = "dangerously_permit_weak_or_unset_master_key";
const PUBLICLY_KNOWN_MASTER_KEY: &str = "sk-1234";

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum UnsafeReason {
    #[strum(
        to_string = "no master key is set, so every request would be accepted without authentication."
    )]
    NotSet,
    #[strum(to_string = "the master key is empty.")]
    Empty,
    #[strum(to_string = "the master key is a publicly known default.")]
    PubliclyKnown,
}

impl UnsafeReason {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::NotSet => "not_set",
            Self::Empty => "empty",
            Self::PubliclyKnown => "publicly_known",
        }
    }

    pub(crate) const fn unsafe_state(self) -> &'static str {
        match self {
            Self::NotSet => {
                "no master key, so every request would be accepted without authentication"
            }
            Self::Empty => "an empty master key",
            Self::PubliclyKnown => "a publicly known master key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyOrigin {
    ConfigFile { path: Option<String> },
    Environment,
}

#[derive(Debug, Clone)]
pub struct StoredSecretsMigration {
    pub from_master_key: SecretString,
    pub encrypted_value_count: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Refusal {
    pub reason: UnsafeReason,
    pub origin: KeyOrigin,
    pub environment_variable_is_set: bool,
    pub migration: Option<StoredSecretsMigration>,
}

#[derive(Debug, Clone)]
pub enum BootVerdict {
    Safe,
    Allowed { reason: UnsafeReason },
    Refused(Refusal),
}

#[derive(Debug, Clone, Copy)]
pub struct OverrideFlags {
    pub env_is_on: bool,
    pub setting_is_on: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DatabaseState {
    pub salt_key_is_set: bool,
    pub configured: bool,
}

#[derive(Debug)]
pub struct BootInput {
    pub master_key: Option<SecretString>,
    pub environment_master_key: Option<SecretString>,
    pub master_key_in_general_settings: bool,
    pub config_file_path: Option<String>,
    pub override_flags: OverrideFlags,
    pub database: DatabaseState,
}

#[must_use]
pub fn boot_verdict(input: BootInput) -> BootVerdict {
    let Some(reason) = unsafe_reason(input.master_key.as_ref()) else {
        return BootVerdict::Safe;
    };
    if input.override_flags.env_is_on || input.override_flags.setting_is_on {
        return BootVerdict::Allowed { reason };
    }
    let relays_environment = input
        .master_key
        .as_ref()
        .zip(input.environment_master_key.as_ref())
        .is_some_and(|(master_key, environment_master_key)| {
            master_key.expose_secret() == environment_master_key.expose_secret()
        });
    let origin = if input.master_key_in_general_settings && !relays_environment {
        KeyOrigin::ConfigFile {
            path: input.config_file_path,
        }
    } else {
        KeyOrigin::Environment
    };
    let environment_variable_is_set = input.environment_master_key.is_some();
    let migration = if input.database.salt_key_is_set || !input.database.configured {
        None
    } else {
        input
            .master_key
            .map(|from_master_key| StoredSecretsMigration {
                from_master_key,
                encrypted_value_count: None,
            })
    };
    BootVerdict::Refused(Refusal {
        reason,
        origin,
        environment_variable_is_set,
        migration,
    })
}

fn unsafe_reason(master_key: Option<&SecretString>) -> Option<UnsafeReason> {
    let Some(configured) = master_key else {
        return Some(UnsafeReason::NotSet);
    };
    let stripped = configured.expose_secret().trim_matches(STRIP_SPACE);
    if stripped.is_empty() {
        return Some(UnsafeReason::Empty);
    }
    if stripped == PUBLICLY_KNOWN_MASTER_KEY {
        return Some(UnsafeReason::PubliclyKnown);
    }
    None
}

// str.strip() in Python also removes U+001C to U+001F, which char::is_whitespace keeps
pub(crate) const STRIP_SPACE: [char; 29] = [
    '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ',
    '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
    '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}',
    '\u{202f}', '\u{205f}', '\u{3000}',
];
