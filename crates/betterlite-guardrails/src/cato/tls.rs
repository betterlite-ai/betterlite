use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::tls::{Verification, VerifySetting};

use super::WsSsl;

const SSL_VERIFY_VARIABLE: &str = "SSL_VERIFY";

pub(super) fn resolve_verification<L>(
    ssl_verify: Option<&VerifySetting>,
    lookup: &L,
) -> Verification
where
    L: EnvironmentLookup,
{
    let text = match ssl_verify {
        Some(VerifySetting::Bool(false)) => return Verification::Disabled,
        Some(VerifySetting::Bool(true)) => return ca_verification(lookup),
        None => environment_value(lookup, SSL_VERIFY_VARIABLE),
        Some(VerifySetting::Path(text)) => Some(text.clone()),
    };
    let Some(value) = text.as_deref() else {
        return ca_verification(lookup);
    };
    match betterlite_env::flag(SSL_VERIFY_VARIABLE, value) {
        Ok(Some(true)) => ca_verification(lookup),
        Ok(Some(false)) => Verification::Disabled,
        Ok(None) | Err(_) => Verification::CaFile(value.to_owned()),
    }
}

pub(super) fn resolve_ws_ssl<L>(
    ssl_verify: Option<&VerifySetting>,
    ws_api_base: &str,
    lookup: &L,
) -> Option<WsSsl>
where
    L: EnvironmentLookup,
{
    let setting = ssl_verify?;
    if !ws_api_base.starts_with("wss://") {
        return None;
    }
    Some(match resolve_verification(Some(setting), lookup) {
        Verification::Disabled => WsSsl::Disabled,
        Verification::Enabled => WsSsl::SystemRoots,
        Verification::CaFile(path) => WsSsl::CaFile(path),
    })
}

fn ca_verification<L>(lookup: &L) -> Verification
where
    L: EnvironmentLookup,
{
    let Some(path) = environment_value(lookup, "SSL_CERT_FILE") else {
        return Verification::Enabled;
    };
    Verification::CaFile(path)
}

pub(super) fn environment_value<L>(lookup: &L, variable: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    match lookup.lookup(variable) {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}
