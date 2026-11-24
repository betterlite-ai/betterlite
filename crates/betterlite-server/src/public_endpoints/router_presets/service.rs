use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, Method};
use betterlite_config::sections::public_router_presets::Settings;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use tokio::sync::Mutex;
use url::Url;

use super::catalog::{BUNDLED_CATALOG, PresetCatalog, decode_catalog};
use super::error::RouterPresetsError;
use crate::roi_github::transport::Transport;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AutorouterPresetService<Client> {
    pub(super) client: Client,
    pub(super) url: String,
    pub(super) local_only: bool,
    pub(super) bundled: &'static str,
    pub(super) cache: Mutex<Option<Arc<PresetCatalog>>>,
    pub(super) fetch_lock: Mutex<()>,
}

impl<Client> AutorouterPresetService<Client>
where
    Client: Transport,
{
    #[must_use]
    pub fn new(client: Client, settings: Settings) -> Self {
        Self {
            client,
            url: settings.url,
            local_only: settings.local_only,
            bundled: BUNDLED_CATALOG,
            cache: Mutex::new(None),
            fetch_lock: Mutex::new(()),
        }
    }

    pub async fn presets(&self) -> Result<Arc<PresetCatalog>, RouterPresetsError> {
        let cached = self.cache.lock().await.clone();
        if let Some(presets) = cached {
            return Ok(presets);
        }
        // one resolution per process: a later caller waits here and takes the first answer instead of fetching too
        let _fetching = self.fetch_lock.lock().await;
        let filled = self.cache.lock().await.clone();
        if let Some(presets) = filled {
            return Ok(presets);
        }
        let catalog = Arc::new(self.resolve().await?);
        *self.cache.lock().await = Some(Arc::clone(&catalog));
        Ok(catalog)
    }

    async fn resolve(&self) -> Result<PresetCatalog, RouterPresetsError> {
        if self.local_only {
            return decode_catalog(self.bundled.as_bytes());
        }
        match self.fetch_remote().await {
            Ok(catalog) => Ok(catalog),
            Err(error) => {
                tracing::warn!(
                    url = %self.url,
                    error = %error,
                    "failed to fetch auto-router presets; serving the bundled catalog"
                );
                decode_catalog(self.bundled.as_bytes())
            }
        }
    }

    async fn fetch_remote(&self) -> Result<PresetCatalog, RouterPresetsError> {
        let url =
            Url::parse(&self.url).map_err(|error| RouterPresetsError::Url(Box::new(error)))?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: FETCH_TIMEOUT,
        };
        let response = self
            .client
            .send(request)
            .await
            .map_err(RouterPresetsError::Request)?;
        // httpx's raise_for_status accepts only 2xx, so an unfollowed 3xx falls back too
        if !(200..300).contains(&response.status) {
            return Err(RouterPresetsError::Status {
                status: response.status,
            });
        }
        let catalog = decode_catalog(&response.body)?;
        if catalog.is_empty() {
            return Err(RouterPresetsError::Empty);
        }
        Ok(catalog)
    }
}
