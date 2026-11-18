use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use std::convert::Infallible;
use std::future::ready;

pub(super) struct Media;
impl RemoteMediaFetcher for Media {
    fn convert_url(&self, _url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Ok("data:image/png;base64,aGk=".to_owned()))
    }
    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Ok(vec![
            "data:image/png;base64,aGk=".to_owned();
            urls.len()
        ]))
    }
}
impl FileBlocks for Media {
    type Failure = Infallible;
    fn file_block(&self, element: &Object) -> Result<Object, Self::Failure> {
        Ok(element.clone())
    }
}
