use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

use super::{
    ChatContent, ChatContentError, Mode, RefuseDownloads, TransformedRequest, UrlDownloader,
};

mod tests;

// The data URLs a recording pass found; a plain transform with it sends file_data for every URL it prefetched
pub struct FetchedMedia {
    data: BTreeMap<String, String>,
}

impl UrlDownloader for FetchedMedia {
    // A URL the caller never prefetched fails as LiteLLM's downloads-off answer does, never as a bare file_id
    fn download(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Disabled {
                url: url.to_owned(),
            })
    }
}

// The pass that lists the URLs LiteLLM would download for this request; it answers an empty data URL
#[derive(Default)]
struct RecordDownloads {
    urls: Mutex<Vec<String>>,
}

impl UrlDownloader for RecordDownloads {
    fn download(&self, url: &str) -> Result<String, ImageFetchError> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        Ok(String::new())
    }
}

// With no fetcher, the open URL-download decision refuses every URL before a request opens
impl RemoteMediaFetcher for RefuseDownloads {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Disabled {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.first().map_or_else(
            || Ok(Vec::new()),
            |url| Err(ImageFetchError::Disabled { url: url.clone() }),
        ))
    }
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the media seam lives beside its recording pass and tests; chat_content.rs keeps the type's first impl"
)]
impl ChatContent<'_> {
    // LiteLLM converts each URL in content-item order as its transform walks the user messages
    pub async fn fetch_media<Source>(
        &self,
        fetcher: &Source,
    ) -> Result<FetchedMedia, ChatContentError>
    where
        Source: RemoteMediaFetcher + Sync,
    {
        let recorded = RecordDownloads::default();
        let recording = ChatContent {
            model: self.model,
            messages: self.messages,
            optional_params: self.optional_params,
            provider: self.provider,
            api_base: self.api_base,
            global_api_base: self.global_api_base,
            openai_system_messages_first: self.openai_system_messages_first,
            environment: self.environment,
            downloads: &recorded,
        };
        let record_error = recording.transformed_messages();
        let mut data = BTreeMap::new();
        for url in recorded
            .urls
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
        {
            let data_url = fetcher.convert_url(&url).await?;
            data.insert(url, data_url);
        }
        // a later content item's missing-file error waits until the URLs before it downloaded, as LiteLLM's loop does
        record_error?;
        Ok(FetchedMedia { data })
    }

    // The body the plain transform builds once every URL it would download is already in hand
    pub async fn transform_with_media<Source>(
        &self,
        mode: Mode,
        fetcher: &Source,
    ) -> Result<TransformedRequest, ChatContentError>
    where
        Source: RemoteMediaFetcher + Sync,
    {
        let media = self.fetch_media(fetcher).await?;
        let replay = ChatContent {
            model: self.model,
            messages: self.messages,
            optional_params: self.optional_params,
            provider: self.provider,
            api_base: self.api_base,
            global_api_base: self.global_api_base,
            openai_system_messages_first: self.openai_system_messages_first,
            environment: self.environment,
            downloads: &media,
        };
        replay.transform(mode)
    }
}
