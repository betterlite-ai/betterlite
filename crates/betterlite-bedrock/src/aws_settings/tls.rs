use std::collections::BTreeMap;

use betterlite_config::{
    tls::{TlsPolicy, TlsPolicyError, Verification, VerifySetting},
    types::Config,
};

pub fn resolve_tls(
    config: &Config,
    environment: &BTreeMap<String, String>,
    explicit: Option<&VerifySetting>,
) -> Result<TlsPolicy, TlsPolicyError> {
    let boolean = match explicit {
        Some(VerifySetting::Bool(value)) => Some(*value),
        Some(VerifySetting::Path(_)) | None => None,
    };
    let mut policy = TlsPolicy::resolve(config, environment, boolean)?;
    if let Some(VerifySetting::Path(path)) = explicit {
        policy.verification = Verification::CaFile(path.clone());
        policy.ca_file = Some(path.clone());
    }
    Ok(policy)
}
