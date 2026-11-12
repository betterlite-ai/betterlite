use super::input::Local;
use super::resolver;
use crate::helpers::HelperResult;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::ExposeSecret as _;
use serde_json::json;

pub(super) fn local(input: &Local) -> HelperResult {
    let settings = AwsSettings::from(&input.settings);
    let observed = resolver::credentials(&settings, &input.environment, &input.files, 0)?;
    let source = resolver::source(&observed, &settings);
    let value = observed.credentials;
    Ok(json!({
        "credentials": (
            value.access_key_id.expose_secret().to_owned(),
            value.secret_access_key.expose_secret().to_owned(),
            value.session_token.map(|token| token.expose_secret().to_owned()),
        ),
        "ttl": observed.ttl,
        "source": source,
    }))
}
