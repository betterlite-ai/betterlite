use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use tokio::time::timeout;

use super::{DOWNLOAD_TIMEOUT, Image, UploadError};
use crate::gigachat_files::data_url::extension_of;

const DEFAULT_CONTENT_TYPE: &str = "image/jpeg";

pub struct ImageDownloader<R> {
    pub urls: UserUrlFetcher<R>,
    pub client: ProxiedClient,
}

impl<R> ImageDownloader<R>
where
    R: Resolve,
{
    pub(super) async fn download(&self, image_url: &str) -> Result<Image, UploadError> {
        let attempt = async {
            let fetched = self
                .urls
                .fetch(&self.client, image_url, &HeaderMap::new())
                .await
                .map_err(UploadError::Download)?;
            fetched
                .buffered(self.client.limits.body_bytes)
                .await
                .map_err(|source| UploadError::Transport {
                    step: "download",
                    source,
                })
        };
        // fetch returns at the response head, so the deadline has to cover the body read too
        let reply = timeout(DOWNLOAD_TIMEOUT, attempt)
            .await
            .map_err(|_elapsed| UploadError::DownloadTimeout)??;
        if reply.status >= 400 {
            return Err(UploadError::DownloadStatus(reply.status));
        }
        let content_type = reply
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map_or_else(|| String::from(DEFAULT_CONTENT_TYPE), str::to_owned);
        Ok(Image {
            extension: extension_of(&content_type),
            content_type,
            data: reply.body,
        })
    }
}
