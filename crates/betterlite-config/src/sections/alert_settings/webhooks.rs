use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::alert_settings::types::AlertType;

const ENV_MARKER: &str = "os.environ/";
const WEBHOOKS_PATH: &str = "general_settings.alert_to_webhook_url";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged, expecting = "a webhook URL string or a list of webhook URLs")]
pub enum WebhookUrl {
    Single(String),
    Multiple(Vec<String>),
}

pub fn resolve_alerting_webhooks<L>(
    webhooks: BTreeMap<AlertType, WebhookUrl>,
    lookup: &L,
) -> Result<BTreeMap<AlertType, WebhookUrl>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = BTreeMap::new();
    for (alert_type, webhook) in webhooks {
        let entry = match webhook {
            WebhookUrl::Single(url) => {
                WebhookUrl::Single(resolve_webhook_url(&url, alert_type, None, lookup)?)
            }
            WebhookUrl::Multiple(urls) => WebhookUrl::Multiple(
                urls.iter()
                    .enumerate()
                    .map(|(index, url)| resolve_webhook_url(url, alert_type, Some(index), lookup))
                    .collect::<Result<Vec<String>, ConfigError>>()?,
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
) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    if !url.contains(ENV_MARKER) {
        return Ok(url.to_owned());
    }
    // LiteLLM hands every string containing the marker to get_secret, so a reference must name a set variable
    let name = url.strip_prefix(ENV_MARKER).unwrap_or(url);
    let path = index.map_or_else(
        || format!("{WEBHOOKS_PATH}.{alert_type}"),
        |position| format!("{WEBHOOKS_PATH}.{alert_type}[{position}]"),
    );
    lookup
        .lookup(name)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path,
        })
}
