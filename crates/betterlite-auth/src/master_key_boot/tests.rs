#![cfg(test)]

use std::error::Error as _;
use std::io::{self, BufWriter, Write};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::database_reencryption::PREVIOUS_KEY_VARIABLE;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::counting::{StoredSecretCounter, with_stored_secrets_counted};
use super::enforce::{BootError, BootReporter, OverrideWarning, enforce_boot_verdict};
use super::refusal::{
    GENERATE_MASTER_KEY_COMMAND, MASTER_KEY_ENV_VAR, PRINT_NEW_MASTER_KEY_COMMAND, render_refusal,
};
use super::stderr::announce_on_stderr;
use super::verdict::{
    BootInput, BootVerdict, DatabaseState, KeyOrigin, MASTER_KEY_SETTING, OVERRIDE_SETTING,
    OverrideFlags, Refusal, StoredSecretsMigration, UnsafeReason, boot_verdict,
};

const GOLDEN_MISSING_KEY: &str = concat!(
    "betterlite refused to start: no master key is set, so every request would be accepted without authentication.\n",
    "Neither general_settings.master_key nor the LITELLM_MASTER_KEY environment variable is set.\n",
    "\n",
    "1. Generate a key and save it to .env:\n",
    "     echo \"LITELLM_MASTER_KEY=sk-$(openssl rand -hex 32)\" | tee -a .env\n",
    "   Not using a .env file (docker run, Kubernetes, pip install)? Pass the same value as the\n",
    "   LITELLM_MASTER_KEY environment variable instead.\n",
    "\n",
    "Local development only: set LITELLM_DANGEROUSLY_PERMIT_WEAK_OR_UNSET_MASTER_KEY=true, or\n",
    "general_settings.dangerously_permit_weak_or_unset_master_key: true, to start anyway.",
);

const GOLDEN_MIGRATION_UNCHECKED: &str = concat!(
    "betterlite refused to start: the master key is empty.\n",
    "It comes from general_settings.master_key in /app/config.yaml.\n",
    "\n",
    "Your database could not be checked for values encrypted with this master key,\n",
    "which encrypts stored credentials while LITELLM_SALT_KEY is not set. Replacing the key alone makes them\n",
    "unreadable, so also tell betterlite which key to migrate from:\n",
    "1. Make sure /app/config.yaml reads the key from the environment:\n",
    "     general_settings:\n",
    "       master_key: os.environ/LITELLM_MASTER_KEY\n",
    "2. Save the key to migrate from and a newly generated key to .env:\n",
    "     echo 'LITELLM_MIGRATE_FROM_MASTER_KEY=' | tee -a .env\n",
    "     echo \"LITELLM_MASTER_KEY=sk-$(openssl rand -hex 32)\" | tee -a .env\n",
    "   Not using a .env file (docker run, Kubernetes, pip install)? Pass the same two values as\n",
    "   environment variables instead.\n",
    "3. Start betterlite again. It re-encrypts the stored values with the new key, then logs that\n",
    "   LITELLM_MIGRATE_FROM_MASTER_KEY can be removed. Details: https://docs.litellm.ai/docs/proxy/master_key_rotations#proxy-refuses-to-start\n",
    "\n",
    "Local development only: set LITELLM_DANGEROUSLY_PERMIT_WEAK_OR_UNSET_MASTER_KEY=true, or\n",
    "general_settings.dangerously_permit_weak_or_unset_master_key: true, to start anyway.",
);

const GOLDEN_EXPORTED_KEY_MIGRATION: &str = concat!(
    "betterlite refused to start: the master key is a publicly known default.\n",
    "It comes from the LITELLM_MASTER_KEY environment variable.\n",
    "\n",
    "Your database holds 3 value(s) encrypted with this master key,\n",
    "which encrypts stored credentials while LITELLM_SALT_KEY is not set. Replacing the key alone makes them\n",
    "unreadable, so also tell betterlite which key to migrate from:\n",
    "1. Set the key to migrate from next to LITELLM_MASTER_KEY, wherever that is set (a shell export, your\n",
    "   container or deployment environment, or .env):\n",
    "     LITELLM_MIGRATE_FROM_MASTER_KEY=sk-1234\n",
    "2. Generate a key:\n",
    "     echo \"sk-$(openssl rand -hex 32)\"\n",
    "   Put it in place of the current LITELLM_MASTER_KEY value wherever that is set: a shell export, your\n",
    "   container or deployment environment, or its line in .env. Do not just add it to .env, because a value\n",
    "   already exported in the environment wins over .env.\n",
    "3. Start betterlite again. It re-encrypts the stored values with the new key, then logs that\n",
    "   LITELLM_MIGRATE_FROM_MASTER_KEY can be removed. Details: https://docs.litellm.ai/docs/proxy/master_key_rotations#proxy-refuses-to-start\n",
    "\n",
    "Local development only: set LITELLM_DANGEROUSLY_PERMIT_WEAK_OR_UNSET_MASTER_KEY=true, or\n",
    "general_settings.dangerously_permit_weak_or_unset_master_key: true, to start anyway.",
);

fn key(value: &str) -> SecretString {
    SecretString::from(value)
}

fn boot_input(master_key: Option<&str>) -> BootInput {
    BootInput {
        master_key: master_key.map(key),
        environment_master_key: None,
        master_key_in_general_settings: false,
        config_file_path: None,
        override_flags: OverrideFlags {
            env_is_on: false,
            setting_is_on: false,
        },
        database: DatabaseState {
            salt_key_is_set: false,
            configured: false,
        },
    }
}

fn from_config(mut input: BootInput, path: Option<&str>) -> BootInput {
    input.master_key_in_general_settings = true;
    input.config_file_path = path.map(str::to_owned);
    input
}

fn with_environment(mut input: BootInput, value: &str) -> BootInput {
    input.environment_master_key = Some(key(value));
    input
}

fn with_database(mut input: BootInput, database: DatabaseState) -> BootInput {
    input.database = database;
    input
}

fn default_refusal(
    environment_variable_is_set: bool,
    migration: Option<StoredSecretsMigration>,
) -> Refusal {
    Refusal {
        reason: UnsafeReason::PubliclyKnown,
        origin: KeyOrigin::Environment,
        environment_variable_is_set,
        migration,
    }
}

fn counts(asked: &[String]) -> Vec<&str> {
    asked.iter().map(String::as_str).collect()
}

#[derive(Default)]
struct Recording {
    warnings: Vec<OverrideWarning>,
    announcements: Vec<String>,
}

impl BootReporter for Recording {
    fn warn(&mut self, warning: OverrideWarning) {
        self.warnings.push(warning);
    }

    fn announce(&mut self, message: &str) -> io::Result<()> {
        self.announcements.push(message.to_owned());
        Ok(())
    }
}

struct BrokenStderr;

impl BootReporter for BrokenStderr {
    fn warn(&mut self, _warning: OverrideWarning) {}

    fn announce(&mut self, _message: &str) -> io::Result<()> {
        Err(io::Error::other("stderr is closed"))
    }
}

#[derive(Debug)]
struct FixedCounter {
    count: Option<usize>,
    asked_about: Mutex<Vec<String>>,
}

impl FixedCounter {
    fn new(count: Option<usize>) -> Self {
        Self {
            count,
            asked_about: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked_about.lock().unwrap().clone()
    }
}

#[async_trait]
impl StoredSecretCounter for FixedCounter {
    async fn count_values_encrypted_with(&self, signing_key: &SecretString) -> Option<usize> {
        self.asked_about
            .lock()
            .unwrap()
            .push(signing_key.expose_secret().to_owned());
        self.count
    }
}

#[derive(Clone, Default)]
struct SharedLog(Arc<Mutex<Vec<u8>>>);

impl SharedLog {
    fn contents(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
}

impl Write for SharedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn strong_key() {
    let verdict = boot_verdict(boot_input(Some("sk-qa-9f2c1e7a44b0d3")));
    assert!(matches!(&verdict, BootVerdict::Safe));
    let mut reporter = Recording::default();
    assert!(enforce_boot_verdict(&verdict, &mut reporter).is_ok());
    assert!(reporter.warnings.is_empty());
    assert!(reporter.announcements.is_empty());
}

#[tokio::test]
async fn missing_key_with_secrets() {
    let verdict = boot_verdict(with_database(
        boot_input(None),
        DatabaseState {
            salt_key_is_set: false,
            configured: true,
        },
    ));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("a server without a master key refuses to start");
    };
    assert_eq!(refused.reason, UnsafeReason::NotSet);
    assert_eq!(refused.origin, KeyOrigin::Environment);
    assert!(!refused.environment_variable_is_set);
    assert!(
        refused.migration.is_none(),
        "there is no key to migrate from, so the refusal cannot offer one"
    );
    let secrets_counter = FixedCounter::new(Some(7));
    let after_counting = with_stored_secrets_counted(verdict, &secrets_counter).await;
    assert!(
        secrets_counter.asked().is_empty(),
        "a missing key has no ciphertext to count"
    );
    let BootVerdict::Refused(after) = &after_counting else {
        panic!("counting leaves a refusal a refusal");
    };
    assert!(after.migration.is_none());
    assert_eq!(render_refusal(after), GOLDEN_MISSING_KEY);
}

#[test]
fn environment_placeholder() {
    let input = with_environment(
        from_config(boot_input(Some("sk-1234")), Some("/app/config.yaml")),
        "sk-1234",
    );
    let verdict = boot_verdict(input);
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("the known default is refused");
    };
    assert_eq!(refused.reason, UnsafeReason::PubliclyKnown);
    assert_eq!(
        refused.origin,
        KeyOrigin::Environment,
        "a config value that is the environment value only relays it"
    );
    assert!(refused.environment_variable_is_set);
    let text = render_refusal(refused);
    assert!(text.contains(&format!(
        "It comes from the {MASTER_KEY_ENV_VAR} environment variable."
    )));
    assert!(
        !text.contains("os.environ/"),
        "an environment-sourced refusal asks for no config edit"
    );
    assert!(text.contains(PRINT_NEW_MASTER_KEY_COMMAND));
    assert!(
        !text.contains("tee -a .env"),
        "an exported key is replaced where it is set, not appended to .env"
    );
}

#[test]
fn config_file_is_the_source_when_it_differs_from_the_environment() {
    let input = with_environment(
        from_config(boot_input(Some("sk-1234")), Some("/app/config.yaml")),
        "sk-qa-9f2c1e7a44b0d3",
    );
    let verdict = boot_verdict(input);
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("the known default in config is refused");
    };
    assert_eq!(
        refused.origin,
        KeyOrigin::ConfigFile {
            path: Some("/app/config.yaml".to_owned())
        }
    );
}

#[test]
fn config_source_reports_a_blank_master_key() {
    let verdict = boot_verdict(from_config(boot_input(None), Some("/app/config.yaml")));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("a blank config master key is refused");
    };
    assert_eq!(refused.reason, UnsafeReason::NotSet);
    assert_eq!(
        refused.origin,
        KeyOrigin::ConfigFile {
            path: Some("/app/config.yaml".to_owned())
        },
        "the config sets master_key even when it resolves to nothing"
    );
    assert!(render_refusal(refused).contains(
        "general_settings.master_key in /app/config.yaml is blank, or points at an environment variable that is not set."
    ));
}

#[test]
fn default_key_refusal() {
    let verdict = boot_verdict(from_config(boot_input(Some("sk-1234")), None));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("the published default is refused");
    };
    assert_eq!(refused.reason, UnsafeReason::PubliclyKnown);
    assert_eq!(refused.origin, KeyOrigin::ConfigFile { path: None });
    let text = render_refusal(refused);
    assert!(
        text.starts_with(
            "betterlite refused to start: the master key is a publicly known default.\n"
        )
    );
    assert!(text.contains("general_settings.master_key in your config file."));
    assert!(text.contains("Make sure your config file reads the key from the environment:"));
    assert!(text.contains(&format!(
        "{MASTER_KEY_SETTING}: os.environ/{MASTER_KEY_ENV_VAR}"
    )));
    assert!(text.contains(GENERATE_MASTER_KEY_COMMAND));
    assert!(
        !text.contains(PREVIOUS_KEY_VARIABLE),
        "no database was configured, so there is nothing to migrate"
    );
}

#[rstest]
#[case::empty("")]
#[case::spaces(" \t\n")]
#[case::controls("\u{1c}\u{1f}")]
fn whitespace_key_refused(#[case] master_key: &str) {
    let verdict = boot_verdict(boot_input(Some(master_key)));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("a whitespace-only key is refused");
    };
    assert_eq!(refused.reason, UnsafeReason::Empty);
    assert!(
        render_refusal(refused)
            .starts_with("betterlite refused to start: the master key is empty.\n")
    );
}

#[rstest]
#[case::spaces("  sk-1234\n")]
#[case::tabs("\tsk-1234\t")]
#[case::controls("\u{1c}sk-1234\u{1f}")]
fn sanitized_refusal(#[case] master_key: &str) {
    let verdict = boot_verdict(with_database(
        boot_input(Some(master_key)),
        DatabaseState {
            salt_key_is_set: false,
            configured: true,
        },
    ));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("whitespace around the known default does not hide it");
    };
    assert_eq!(refused.reason, UnsafeReason::PubliclyKnown);
    let text = render_refusal(refused);
    let assignment = format!("{PREVIOUS_KEY_VARIABLE}=\"{master_key}\"");
    assert!(
        text.contains(&assignment),
        "the fix carries the exact key the database may be encrypted with"
    );
    assert!(text.contains(&format!("echo '{assignment}' | tee -a .env")));
    let mut reporter = Recording::default();
    let error = enforce_boot_verdict(&verdict, &mut reporter).unwrap_err();
    assert!(
        !error.to_string().contains("sk-1234"),
        "the raised error never carries the key"
    );
}

#[rstest]
#[case::resembles("sk-12345")]
#[case::digits("1234")]
#[case::suffixed("sk-1234x")]
#[case::random("sk-qa-9f2c1e7a44b0d3")]
fn other_short_key_accepted(#[case] master_key: &str) {
    let verdict = boot_verdict(boot_input(Some(master_key)));
    assert!(
        matches!(&verdict, BootVerdict::Safe),
        "only the stripped known default is refused, no length or entropy rule is added"
    );
}

#[rstest]
#[case::not_set_unset(
    None,
    UnsafeReason::NotSet,
    "no master key, so every request would be accepted without authentication",
    true,
    false
)]
#[case::not_set_setting(
    None,
    UnsafeReason::NotSet,
    "no master key, so every request would be accepted without authentication",
    false,
    true
)]
#[case::empty_unset(Some("   "), UnsafeReason::Empty, "an empty master key", true, false)]
#[case::empty_setting(Some("   "), UnsafeReason::Empty, "an empty master key", false, true)]
#[case::known_unset(
    Some("sk-1234"),
    UnsafeReason::PubliclyKnown,
    "a publicly known master key",
    true,
    false
)]
#[case::known_setting(
    Some("sk-1234"),
    UnsafeReason::PubliclyKnown,
    "a publicly known master key",
    false,
    true
)]
fn development_override_warning(
    #[case] master_key: Option<&str>,
    #[case] expected: UnsafeReason,
    #[case] state: &str,
    #[case] env_is_on: bool,
    #[case] setting_is_on: bool,
) {
    let verdict = boot_verdict(BootInput {
        override_flags: OverrideFlags {
            env_is_on,
            setting_is_on,
        },
        ..boot_input(master_key)
    });
    let BootVerdict::Allowed { reason } = &verdict else {
        panic!("either override lets an unsafe key through");
    };
    assert_eq!(*reason, expected);
    let mut reporter = Recording::default();
    assert!(enforce_boot_verdict(&verdict, &mut reporter).is_ok());
    assert!(
        reporter.announcements.is_empty(),
        "an overridden server announces no fix"
    );
    assert_eq!(reporter.warnings.len(), 1);
    assert_eq!(reporter.warnings[0].setting, OVERRIDE_SETTING);
    assert_eq!(
        reporter.warnings[0].message,
        format!(
            "{OVERRIDE_SETTING} is on, so betterlite is starting with {state}. Never run this outside local development."
        )
    );
}

#[tokio::test]
async fn migration_required() {
    let verdict = boot_verdict(with_database(
        from_config(boot_input(Some("")), Some("/app/config.yaml")),
        DatabaseState {
            salt_key_is_set: false,
            configured: true,
        },
    ));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("an empty key is refused");
    };
    assert_eq!(refused.reason, UnsafeReason::Empty);
    let Some(migration) = &refused.migration else {
        panic!("the database may hold values encrypted with the empty key");
    };
    assert_eq!(migration.from_master_key.expose_secret(), "");
    assert_eq!(migration.encrypted_value_count, None);

    let unreadable = FixedCounter::new(None);
    let unchecked = with_stored_secrets_counted(verdict, &unreadable).await;
    let unreadable_asked = unreadable.asked();
    assert_eq!(counts(&unreadable_asked), [""]);
    let BootVerdict::Refused(unchecked_refusal) = &unchecked else {
        panic!("counting leaves a refusal a refusal");
    };
    assert_eq!(
        render_refusal(unchecked_refusal),
        GOLDEN_MIGRATION_UNCHECKED
    );

    let readable = FixedCounter::new(Some(4));
    let counted = with_stored_secrets_counted(unchecked, &readable).await;
    let readable_asked = readable.asked();
    assert_eq!(counts(&readable_asked), [""]);
    let BootVerdict::Refused(counted_refusal) = &counted else {
        panic!("counting leaves a refusal a refusal");
    };
    let text = render_refusal(counted_refusal);
    assert!(text.contains("holds 4 value(s) encrypted with this master key"));
    assert!(text.contains(&format!("{PREVIOUS_KEY_VARIABLE}=")));
}

#[tokio::test]
async fn database_with_nothing_encrypted_needs_no_migration() {
    let verdict = boot_verdict(with_database(
        boot_input(Some("sk-1234")),
        DatabaseState {
            salt_key_is_set: false,
            configured: true,
        },
    ));
    let secrets_counter = FixedCounter::new(Some(0));
    let after_counting = with_stored_secrets_counted(verdict, &secrets_counter).await;
    let asked = secrets_counter.asked();
    assert_eq!(counts(&asked), ["sk-1234"]);
    let BootVerdict::Refused(refused) = &after_counting else {
        panic!("counting leaves a refusal a refusal");
    };
    assert!(
        refused.migration.is_none(),
        "a database with no stored secret keeps the refusal free of migration steps"
    );
    let text = render_refusal(refused);
    assert!(!text.contains(PREVIOUS_KEY_VARIABLE));
    assert!(!text.contains("master_key_rotations"));
}

#[tokio::test]
async fn salt_key_or_missing_database_skips_the_migration() {
    let salted = boot_verdict(with_database(
        boot_input(Some("sk-1234")),
        DatabaseState {
            salt_key_is_set: true,
            configured: true,
        },
    ));
    let secrets_counter = FixedCounter::new(Some(2));
    let salted_verdict = with_stored_secrets_counted(salted, &secrets_counter).await;
    assert!(
        matches!(&salted_verdict, BootVerdict::Refused(refused) if refused.migration.is_none()),
        "a salt key means the database is not encrypted with the master key"
    );

    let unconfigured = boot_verdict(with_database(
        boot_input(Some("sk-1234")),
        DatabaseState {
            salt_key_is_set: false,
            configured: false,
        },
    ));
    let unconfigured_verdict = with_stored_secrets_counted(unconfigured, &secrets_counter).await;
    assert!(
        matches!(&unconfigured_verdict, BootVerdict::Refused(refused) if refused.migration.is_none()),
        "a server without a database has no ciphertext to migrate"
    );

    let salted_without_database = boot_verdict(with_database(
        boot_input(Some("sk-1234")),
        DatabaseState {
            salt_key_is_set: true,
            configured: false,
        },
    ));
    let skipped_verdict =
        with_stored_secrets_counted(salted_without_database, &secrets_counter).await;
    assert!(
        matches!(&skipped_verdict, BootVerdict::Refused(refused) if refused.migration.is_none()),
        "a salt key skips the migration even when no database is configured"
    );
    assert!(
        secrets_counter.asked().is_empty(),
        "no migration means no read"
    );
}

#[tokio::test]
async fn counting_leaves_safe_and_overridden_verdicts_alone() {
    let secrets_counter = FixedCounter::new(Some(7));
    let safe_verdict = with_stored_secrets_counted(BootVerdict::Safe, &secrets_counter).await;
    assert!(matches!(&safe_verdict, BootVerdict::Safe));
    let allowed_verdict = with_stored_secrets_counted(
        BootVerdict::Allowed {
            reason: UnsafeReason::Empty,
        },
        &secrets_counter,
    )
    .await;
    assert!(matches!(
        &allowed_verdict,
        BootVerdict::Allowed {
            reason: UnsafeReason::Empty
        }
    ));
    assert!(secrets_counter.asked().is_empty());
}

#[test]
fn exported_key_is_not_appended_to_the_env_file() {
    let migration = StoredSecretsMigration {
        from_master_key: key("sk-1234"),
        encrypted_value_count: Some(3),
    };
    let text = render_refusal(&default_refusal(true, Some(migration)));
    assert_eq!(text, GOLDEN_EXPORTED_KEY_MIGRATION);
}

#[test]
fn refusal_is_announced_then_raised() {
    let verdict = BootVerdict::Refused(default_refusal(false, None));
    let mut reporter = Recording::default();
    let error = enforce_boot_verdict(&verdict, &mut reporter).unwrap_err();
    assert!(matches!(
        &error,
        BootError::Refused {
            reason: UnsafeReason::PubliclyKnown
        }
    ));
    let BootVerdict::Refused(refused) = &verdict else {
        panic!("the verdict is a refusal");
    };
    let text = render_refusal(refused);
    assert_eq!(reporter.announcements, [format!("\n{text}\n\n")]);
}

#[test]
fn announcement_failure_is_reported() {
    let verdict = BootVerdict::Refused(default_refusal(false, None));
    let error = enforce_boot_verdict(&verdict, &mut BrokenStderr).unwrap_err();
    assert!(matches!(
        &error,
        BootError::Announce {
            reason: UnsafeReason::PubliclyKnown,
            ..
        }
    ));
    assert!(error.source().is_some(), "the write failure is the cause");
}

#[test]
fn announce_flushes_buffered_stdout_first() {
    let log = SharedLog::default();
    let mut stdout = BufWriter::new(log.clone());
    stdout.write_all(b"buffered stdout").unwrap();
    assert!(
        log.contents().is_empty(),
        "stdout is still buffered before the fix goes out"
    );
    let mut stderr = log.clone();
    announce_on_stderr("THE FIX", &mut stdout, &mut stderr).unwrap();
    assert_eq!(log.contents(), b"buffered stdoutTHE FIX");
}

#[test]
fn unsafe_reasons_name_themselves() {
    assert_eq!(
        UnsafeReason::NotSet.to_string(),
        "no master key is set, so every request would be accepted without authentication."
    );
    assert_eq!(UnsafeReason::Empty.to_string(), "the master key is empty.");
    assert_eq!(
        UnsafeReason::PubliclyKnown.to_string(),
        "the master key is a publicly known default."
    );
    assert_eq!(UnsafeReason::NotSet.wire_name(), "not_set");
    assert_eq!(UnsafeReason::Empty.wire_name(), "empty");
    assert_eq!(UnsafeReason::PubliclyKnown.wire_name(), "publicly_known");
}
