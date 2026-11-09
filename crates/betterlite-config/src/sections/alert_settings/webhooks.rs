use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::alert_settings::flags::ENV_MARKER;
use crate::sections::alert_settings::types::AlertType;

const WEBHOOKS_PATH: &str = "general_settings.alert_to_webhook_url";

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged, expecting = "a webhook URL string or a list of webhook URLs")]
pub enum RawWebhookUrl {
    Single(String),
    Multiple(Vec<String>),
}

// a webhook URL carries its access token in the path, so it never leaves its secret wrapper
#[derive(Clone, Debug)]
pub enum WebhookUrl {
    Single(SecretString),
    Multiple(Vec<SecretString>),
}

pub fn resolve_alerting_webhooks<L>(
    webhooks: BTreeMap<AlertType, RawWebhookUrl>,
    lookup: &L,
) -> Result<BTreeMap<AlertType, WebhookUrl>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = BTreeMap::new();
    for (alert_type, webhook) in webhooks {
        let entry = match webhook {
            RawWebhookUrl::Single(url) => {
                WebhookUrl::Single(resolve_webhook_url(&url, alert_type, None, lookup)?)
            }
            RawWebhookUrl::Multiple(urls) => WebhookUrl::Multiple(
                urls.iter()
                    .enumerate()
                    .map(|(index, url)| resolve_webhook_url(url, alert_type, Some(index), lookup))
                    .collect::<Result<Vec<SecretString>, ConfigError>>()?,
            ),
        };
        resolved.insert(alert_type, entry);
    }
    Ok(resolved)
}

fn resolve_webhook_url<L>(
    url: &str,
    alert_type: AlertType,
    index: Option<usize>,
    lookup: &L,
) -> Result<SecretString, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = index.map_or_else(
        || format!("{WEBHOOKS_PATH}.{alert_type}"),
        |position| format!("{WEBHOOKS_PATH}.{alert_type}[{position}]"),
    );
    let Some((before, name)) = url.split_once(ENV_MARKER) else {
        return Ok(SecretString::from(url));
    };
    if !before.is_empty() {
        // the error names only the variable, never the URL that holds it
        return Err(ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path,
        });
    }
    let value = lookup
        .lookup(name)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path,
        })?;
    Ok(SecretString::from(value))
}
