use std::collections::{BTreeMap, HashMap};
use std::str;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::alert_settings::types::AlertType;
use futures_util::future::join_all;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use url::{ParseError, Url};

use crate::alert_settings::records::AlertQueueItem;
use crate::alert_teams::{MS_TEAMS_ALERTING_DESTINATION, build_ms_teams_payload};
use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

pub mod alert_delivery;

mod tests;

pub const ALERT_HEADER_NAME: &str = "Content-type";
pub const ALERT_HEADER_VALUE: &str = "application/json";

#[derive(Clone, Debug)]
pub struct SquashedAlert {
    pub item: AlertQueueItem,
    pub count: u64,
}

// The URL carries the webhook token, so the key borrows it for the duration of one squash only
#[derive(Eq, Hash, PartialEq)]
struct SquashKey<'item> {
    url: &'item str,
    alert_type: AlertType,
    text: &'item str,
}

#[must_use]
pub fn alert_headers() -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    headers.insert(ALERT_HEADER_NAME.to_owned(), ALERT_HEADER_VALUE.to_owned());
    headers
}

#[must_use]
pub fn squash_payloads(queue: &[AlertQueueItem]) -> Vec<SquashedAlert> {
    let mut positions: HashMap<SquashKey<'_>, usize> = HashMap::new();
    let mut squashed: Vec<SquashedAlert> = Vec::new();
    for item in queue {
        let key = SquashKey {
            url: item.url.expose_secret(),
            alert_type: item.alert_type,
            text: &item.payload.text,
        };
        if let Some(position) = positions.get(&key) {
            if let Some(alert) = squashed.get_mut(*position) {
                alert.count = alert.count.saturating_add(1);
            }
            continue;
        }
        positions.insert(key, squashed.len());
        squashed.push(SquashedAlert {
            item: item.clone(),
            count: 1,
        });
    }
    squashed
}

#[derive(Debug, thiserror::Error)]
enum WebhookRequestError {
    #[error("the webhook URL is not a valid URL")]
    Url(#[source] ParseError),
    #[error("the webhook header name {name} is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the webhook header {name} holds a character a header value cannot carry")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the webhook payload could not be written as JSON")]
    Encode(#[source] serde_json::Error),
}

#[derive(Serialize)]
struct WebhookText<'text> {
    text: &'text str,
}

fn webhook_request(item: &AlertQueueItem, text: &str) -> Result<PostRequest, WebhookRequestError> {
    let url = Url::parse(item.url.expose_secret()).map_err(WebhookRequestError::Url)?;
    let mut headers = HeaderMap::new();
    for (name, value) in &item.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            WebhookRequestError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| WebhookRequestError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        headers.insert(header_name, header_value);
    }
    let body = if item.format.as_deref() == Some(MS_TEAMS_ALERTING_DESTINATION) {
        serde_json::to_vec(&build_ms_teams_payload(text)).map_err(WebhookRequestError::Encode)?
    } else {
        serde_json::to_vec(&WebhookText { text }).map_err(WebhookRequestError::Encode)?
    };
    Ok(PostRequest {
        url,
        headers,
        body,
        deadline: DELIVERY_DEADLINE,
    })
}

async fn deliver_webhook<D>(delivery: &D, item: &AlertQueueItem, text: &str) -> bool
where
    D: Delivery,
{
    let request = match webhook_request(item, text) {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(
                %error,
                alert_type = %item.alert_type,
                "the webhook alert request could not be built"
            );
            return false;
        }
    };
    match delivery.post(request).await {
        Ok(response) if response.status == 200 => true,
        Ok(response) => {
            tracing::debug!(
                status = response.status,
                alert_type = %item.alert_type,
                detail = str::from_utf8(&response.body).unwrap_or_default(),
                "the webhook answered with a status other than 200; the alert was not delivered"
            );
            false
        }
        Err(error) => {
            tracing::debug!(
                %error,
                alert_type = %item.alert_type,
                "the webhook alert could not be delivered"
            );
            false
        }
    }
}

#[must_use]
pub async fn send_to_webhook<D>(
    delivery: &D,
    log_to_console: bool,
    item: &AlertQueueItem,
    count: u64,
) -> bool
where
    D: Delivery,
{
    let text = if count == 1 {
        item.payload.text.clone()
    } else {
        format!("[Num Alerts: {count}]\n\n{}", item.payload.text)
    };
    let delivered = deliver_webhook(delivery, item, &text).await;
    if log_to_console {
        tracing::warn!(text = %text, "alert payload");
    }
    delivered
}

pub struct WebhookBatch<'delivery, D> {
    pub delivery: &'delivery D,
    pub log_to_console: bool,
}

impl<D> BatchDelivery<AlertQueueItem> for WebhookBatch<'_, D>
where
    D: Delivery,
{
    async fn deliver(
        &self,
        batch: &[AlertQueueItem],
        settled: &AtomicUsize,
    ) -> Result<Vec<AlertQueueItem>, BatchCancelled<AlertQueueItem>> {
        let squashed = squash_payloads(batch);
        join_all(squashed.iter().map(|alert| {
            send_to_webhook(self.delivery, self.log_to_console, &alert.item, alert.count)
        }))
        .await;
        settled.fetch_add(batch.len(), Ordering::SeqCst);
        Ok(Vec::new())
    }
}
