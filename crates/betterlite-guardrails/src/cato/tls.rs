use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::tls::{Verification, VerifySetting};

use super::WsSsl;

const SSL_VERIFY_VARIABLE: &str = "SSL_VERIFY";
const SSL_CERT_FILE_VARIABLE: &str = "SSL_CERT_FILE";

pub(super) fn resolve_verification<L>(
    ssl_verify: Option<&VerifySetting>,
    lookup: &L,
) -> Verification
where
    L: EnvironmentLookup,
{
    let text = match ssl_verify {
        Some(VerifySetting::Bool(false)) => return Verification::Disabled,
        Some(VerifySetting::Bool(true)) | None => environment(lookup, SSL_VERIFY_VARIABLE),
        Some(VerifySetting::Path(text)) => Some(text.clone()),
    };
    verification_from(text.as_deref(), lookup)
}

pub(super) fn resolve_ws_ssl<L>(
    ssl_verify: Option<&VerifySetting>,
    ws_api_base: &str,
    lookup: &L,
) -> Option<WsSsl>
where
    L: EnvironmentLookup,
{
    let ssl_verify = ssl_verify?;
    if !ws_api_base.starts_with("wss://") {
        return None;
    }
    Some(match resolve_verification(Some(ssl_verify), lookup) {
        Verification::Disabled => WsSsl::Disabled,
        Verification::Enabled => WsSsl::SystemRoots,
        Verification::CaFile(path) => WsSsl::CaFile(path),
    })
}

fn verification_from<L>(text: Option<&str>, lookup: &L) -> Verification
where
    L: EnvironmentLookup,
{
    match text {
        Some(text) => match betterlite_env::flag(SSL_VERIFY_VARIABLE, text)
            .ok()
            .flatten()
        {
            Some(true) => ca_verification(lookup),
            Some(false) => Verification::Disabled,
            None => Verification::CaFile(text.to_owned()),
        },
        None => ca_verification(lookup),
    }
}

fn ca_verification<L>(lookup: &L) -> Verification
where
    L: EnvironmentLookup,
{
    match environment(lookup, SSL_CERT_FILE_VARIABLE) {
        Some(path) => Verification::CaFile(path),
        None => Verification::Enabled,
    }
}

fn environment<L>(lookup: &L, variable: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    let value = lookup.lookup(variable)?;
    if value.is_empty() { None } else { Some(value) }
}
