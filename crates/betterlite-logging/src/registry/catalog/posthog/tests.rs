#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::registry::catalog::{CatalogError, CatalogInputs};

use super::build;

fn inputs(pairs: &[(&str, &str)]) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<String, String>>(),
        ),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

#[tokio::test]
async fn callback_logger_builds_the_posthog_logger() {
    let inputs = inputs(&[("POSTHOG_API_KEY", "phk-test")]);
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "posthog");
    assert_eq!(inputs.tracker.len(), 1);
}

#[tokio::test]
async fn callback_logger_reports_a_missing_api_key() {
    let inputs = inputs(&[]);
    let error = build(&inputs).err().expect("the key is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("POSTHOG_API_KEY")
    ));
}

#[tokio::test(start_paused = true)]
async fn callback_logger_stops_its_shutdown_task() {
    let inputs = inputs(&[("POSTHOG_API_KEY", "phk-test")]);
    let _logger = build(&inputs).expect("the logger builds");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    let awaited = timeout(Duration::from_secs(1), inputs.tracker.wait()).await;
    awaited.expect("the shutdown task ends");
}
