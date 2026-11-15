use betterlite_db::database_reencryption::{PREVIOUS_KEY_VARIABLE, SALT_KEY_ENV_VAR};
use secrecy::ExposeSecret as _;

use super::verdict::{
    KeyOrigin, MASTER_KEY_SETTING, OVERRIDE_SETTING, Refusal, StoredSecretsMigration, UnsafeReason,
    is_blank,
};

pub const MASTER_KEY_ENV_VAR: &str = "LITELLM_MASTER_KEY";
pub const OVERRIDE_ENV_VAR: &str = "LITELLM_DANGEROUSLY_PERMIT_WEAK_OR_UNSET_MASTER_KEY";
pub const ROTATION_DOCS_URL: &str =
    "https://docs.litellm.ai/docs/proxy/master_key_rotations#proxy-refuses-to-start";
pub const GENERATE_MASTER_KEY_COMMAND: &str =
    "echo \"LITELLM_MASTER_KEY=sk-$(openssl rand -hex 32)\" | tee -a .env";
pub const PRINT_NEW_MASTER_KEY_COMMAND: &str = "echo \"sk-$(openssl rand -hex 32)\"";
const CONFIG_LABEL: &str = "your config file";

#[must_use]
pub fn render_refusal(refusal: &Refusal) -> String {
    let reason = refusal.reason;
    let source = source_line(refusal);
    let headline = format!("betterlite refused to start: {reason}\n{source}");
    let steps = fix_steps(refusal);
    let hint = override_hint();
    [headline, steps, hint].join("\n\n")
}

fn source_line(refusal: &Refusal) -> String {
    match (&refusal.origin, refusal.reason) {
        (KeyOrigin::ConfigFile { path }, UnsafeReason::NotSet) => {
            let label = path.as_deref().unwrap_or(CONFIG_LABEL);
            format!(
                "general_settings.{MASTER_KEY_SETTING} in {label} is blank, or points at an environment variable that is not set."
            )
        }
        (KeyOrigin::ConfigFile { path }, _) => {
            let label = path.as_deref().unwrap_or(CONFIG_LABEL);
            format!("It comes from general_settings.{MASTER_KEY_SETTING} in {label}.")
        }
        (KeyOrigin::Environment, UnsafeReason::NotSet) => {
            format!(
                "Neither general_settings.{MASTER_KEY_SETTING} nor the {MASTER_KEY_ENV_VAR} environment variable is set."
            )
        }
        (KeyOrigin::Environment, _) => {
            format!("It comes from the {MASTER_KEY_ENV_VAR} environment variable.")
        }
    }
}

fn fix_steps(refusal: &Refusal) -> String {
    let mut steps = config_steps(&refusal.origin);
    steps.extend(key_steps(refusal));
    let numbered = (1..)
        .zip(steps.iter())
        .map(|(number, step)| format!("{number}. {step}"))
        .collect::<Vec<_>>()
        .join("\n");
    let Some(migration) = &refusal.migration else {
        return numbered;
    };
    let lead = migration_lead(migration);
    format!("{lead}\n{numbered}")
}

fn config_steps(origin: &KeyOrigin) -> Vec<String> {
    match origin {
        KeyOrigin::ConfigFile { path } => {
            let label = path.as_deref().unwrap_or(CONFIG_LABEL);
            vec![format!(
                "Make sure {label} reads the key from the environment:\n     general_settings:\n       {MASTER_KEY_SETTING}: os.environ/{MASTER_KEY_ENV_VAR}"
            )]
        }
        KeyOrigin::Environment => Vec::new(),
    }
}

fn key_steps(refusal: &Refusal) -> Vec<String> {
    let Some(migration) = &refusal.migration else {
        return if refusal.environment_variable_is_set {
            vec![replace_exported_key_step()]
        } else {
            vec![save_key_step()]
        };
    };
    if refusal.environment_variable_is_set {
        return vec![
            migrate_next_to_variable_step(migration),
            replace_exported_key_step(),
            restart_to_migrate_step(),
        ];
    }
    vec![
        save_migrated_keys_step(migration),
        restart_to_migrate_step(),
    ]
}

fn save_key_step() -> String {
    format!(
        "Generate a key and save it to .env:\n     {GENERATE_MASTER_KEY_COMMAND}\n   Not using a .env file (docker run, Kubernetes, pip install)? Pass the same value as the\n   {MASTER_KEY_ENV_VAR} environment variable instead."
    )
}

fn replace_exported_key_step() -> String {
    format!(
        "Generate a key:\n     {PRINT_NEW_MASTER_KEY_COMMAND}\n   Put it in place of the current {MASTER_KEY_ENV_VAR} value wherever that is set: a shell export, your\n   container or deployment environment, or its line in .env. Do not just add it to .env, because a value\n   already exported in the environment wins over .env."
    )
}

fn restart_to_migrate_step() -> String {
    format!(
        "Start betterlite again. It re-encrypts the stored values with the new key, then logs that\n   {PREVIOUS_KEY_VARIABLE} can be removed. Details: {ROTATION_DOCS_URL}"
    )
}

fn migrate_next_to_variable_step(migration: &StoredSecretsMigration) -> String {
    let assignment = migrate_from_assignment(migration);
    format!(
        "Set the key to migrate from next to {MASTER_KEY_ENV_VAR}, wherever that is set (a shell export, your\n   container or deployment environment, or .env):\n     {assignment}"
    )
}

fn save_migrated_keys_step(migration: &StoredSecretsMigration) -> String {
    let assignment = migrate_from_assignment(migration);
    format!(
        "Save the key to migrate from and a newly generated key to .env:\n     echo '{assignment}' | tee -a .env\n     {GENERATE_MASTER_KEY_COMMAND}\n   Not using a .env file (docker run, Kubernetes, pip install)? Pass the same two values as\n   environment variables instead."
    )
}

fn migrate_from_assignment(migration: &StoredSecretsMigration) -> String {
    let key = migration.from_master_key.expose_secret();
    let value = if key == key.trim_matches(is_blank) {
        key.to_owned()
    } else {
        format!("\"{key}\"")
    };
    format!("{PREVIOUS_KEY_VARIABLE}={value}")
}

fn migration_lead(migration: &StoredSecretsMigration) -> String {
    let found = migration.encrypted_value_count.map_or_else(
        || "could not be checked for values".to_owned(),
        |count| format!("holds {count} value(s)"),
    );
    format!(
        "Your database {found} encrypted with this master key,\nwhich encrypts stored credentials while {SALT_KEY_ENV_VAR} is not set. Replacing the key alone makes them\nunreadable, so also tell betterlite which key to migrate from:"
    )
}

fn override_hint() -> String {
    format!(
        "Local development only: set {OVERRIDE_ENV_VAR}=true, or\ngeneral_settings.{OVERRIDE_SETTING}: true, to start anyway."
    )
}
