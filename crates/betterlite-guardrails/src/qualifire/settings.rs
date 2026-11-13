use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::QualifireError;
use super::init::QualifireArgs;

pub const DEFAULT_API_BASE: &str = "https://proxy.qualifire.ai";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

pub trait SecretLookup {
    fn secret(&self, name: &str) -> Option<SecretString>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Check {
    PromptInjections,
    Hallucinations,
    Grounding,
    Pii,
    ContentModeration,
    ToolSelectionQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFlagged {
    Block,
    Monitor,
}

impl OnFlagged {
    pub const fn parse(value: &str) -> Option<Self> {
        match value {
            "block" => Some(Self::Block),
            "monitor" => Some(Self::Monitor),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualifireSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
    pub evaluation_id: Option<String>,
    pub checks: BTreeSet<Check>,
    pub assertions: Vec<String>,
    pub on_flagged: OnFlagged,
    pub timeout: Duration,
}

pub fn resolve(
    args: &QualifireArgs,
    secrets: &dyn SecretLookup,
    environment: &dyn EnvironmentLookup,
) -> Result<QualifireSettings, QualifireError> {
    let api_key = secret_value(
        args.api_key.clone(),
        "QUALIFIRE_API_KEY",
        secrets,
        environment,
    );
    let api_base = text_value(
        args.api_base.clone(),
        "QUALIFIRE_BASE_URL",
        secrets,
        environment,
    )
    .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let evaluation_id = args.evaluation_id.clone().filter(|id| !id.is_empty());
    let assertions = args.assertions.clone().unwrap_or_default();
    let mut checks = checks_from_args(args);
    if checks.is_empty() && assertions.is_empty() && evaluation_id.is_none() {
        checks.insert(Check::PromptInjections);
    }
    let on_flagged = match args.on_flagged.as_deref() {
        None => OnFlagged::Block,
        Some(text) => OnFlagged::parse(text)
            .ok_or_else(|| QualifireError::UnsupportedOnFlagged(text.to_owned()))?,
    };
    let timeout = match args.timeout {
        Some(seconds) => Duration::try_from_secs_f64(seconds)
            .map_err(|source| QualifireError::InvalidTimeout { source })?,
        None => DEFAULT_TIMEOUT,
    };
    Ok(QualifireSettings {
        api_key,
        api_base,
        evaluation_id,
        checks,
        assertions,
        on_flagged,
        timeout,
    })
}

fn checks_from_args(args: &QualifireArgs) -> BTreeSet<Check> {
    let mut checks = BTreeSet::new();
    insert_flag(&mut checks, Check::PromptInjections, args.prompt_injections);
    insert_flag(
        &mut checks,
        Check::Hallucinations,
        args.hallucinations_check,
    );
    insert_flag(&mut checks, Check::Grounding, args.grounding_check);
    insert_flag(&mut checks, Check::Pii, args.pii_check);
    insert_flag(
        &mut checks,
        Check::ContentModeration,
        args.content_moderation_check,
    );
    insert_flag(
        &mut checks,
        Check::ToolSelectionQuality,
        args.tool_selection_quality_check,
    );
    checks
}

pub(super) fn insert_flag(checks: &mut BTreeSet<Check>, check: Check, enabled: Option<bool>) {
    if enabled.unwrap_or(false) {
        checks.insert(check);
    }
}

// an unset, empty or null value falls through to the next source, and the last one is None
fn secret_value(
    configured: Option<SecretString>,
    name: &str,
    secrets: &dyn SecretLookup,
    environment: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    configured
        .or_else(|| {
            secrets
                .secret(name)
                .filter(|secret| !secret.expose_secret().is_empty())
        })
        .or_else(|| {
            environment
                .lookup(name)
                .filter(|text| !text.is_empty())
                .map(SecretString::from)
        })
}

fn text_value(
    configured: Option<String>,
    name: &str,
    secrets: &dyn SecretLookup,
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    configured
        .or_else(|| {
            secrets
                .secret(name)
                .map(|secret| secret.expose_secret().to_owned())
                .filter(|text| !text.is_empty())
        })
        .or_else(|| environment.lookup(name).filter(|text| !text.is_empty()))
}
