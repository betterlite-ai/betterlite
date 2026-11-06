use std::borrow::Cow;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::sandbox::ContainerHandle;
use http::Method;
use url::Url;

use super::super::helpers::request::{SandboxRef, lifecycle_headers};
use super::super::helpers::{
    OpenSandbox, OpenSandboxError, SandboxHttp, SandboxRequest, check_status,
};
use super::{ContainerRef, DeleteSandbox, LifecycleError};

impl<Http, Environment, Clock> OpenSandbox<Http, Environment, Clock>
where
    Http: SandboxHttp,
    Environment: EnvironmentLookup + Sync,
    Clock: Sync,
{
    pub async fn adelete_sandbox(
        &self,
        request: DeleteSandbox<'_>,
        client: Option<&Http>,
    ) -> Result<bool, LifecycleError> {
        let container: Cow<'_, ContainerHandle> = match request.container {
            ContainerRef::Cached(handle) => Cow::Borrowed(&handle.container),
            ContainerRef::Handle(handle) => Cow::Borrowed(handle),
            ContainerRef::Id(id) => self.as_handle(SandboxRef::Id(id), request.api_base)?,
        };
        let api_base = match container.hidden_params.api_base.as_deref() {
            Some(base) => base.to_owned(),
            None => self.api_base(request.api_base)?,
        };
        let api_key = self.api_key(request.api_key, &container);
        let url = Url::parse(&format!("{api_base}/sandboxes/{}", container.id))
            .map_err(|source| OpenSandboxError::Url { source })?;
        let response = client
            .unwrap_or(&self.http)
            .send(SandboxRequest {
                method: Method::DELETE,
                url,
                headers: lifecycle_headers(&api_key)?,
                body: Vec::new(),
                timeout: None,
            })
            .await?;
        if response.status == 404 {
            return Ok(false);
        }
        check_status(response.status)?;
        Ok(true)
    }
}
