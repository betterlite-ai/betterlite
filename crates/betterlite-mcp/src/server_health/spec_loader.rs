use std::future::Future;
use std::sync::Arc;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;

use super::ports::SpecLoader;
use crate::openapi_tools::error::OpenApiError;
use crate::openapi_tools::spec_loading::load_openapi_spec_async;

pub struct OpenApiSpecLoader<R> {
    pub fetcher: UserUrlFetcher<R>,
    pub client: Arc<ProxiedClient>,
}

impl<R> SpecLoader for OpenApiSpecLoader<R>
where
    R: Resolve,
{
    fn load(
        &self,
        spec_path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<(), OpenApiError>> + Send {
        async move {
            load_openapi_spec_async(&self.fetcher, &self.client, spec_path, Some(max_bytes))
                .await?;
            Ok(())
        }
    }
}
