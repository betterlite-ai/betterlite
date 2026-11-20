use secrecy::SecretString;

use super::config::{BaseVideoConfig, Headers, Params, VideoEnvironment, VideoRequestContext};
use super::error::{VideoClientError, VideoEditTransportError};
use super::seams::{Selected, VideoClientSource, VideoErrorConvert, VideoLoggingHandle};
use super::wire::{VideoParams, header_pairs};
use crate::api::vector_store_http_write::error::FailureFacts;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

// The four LiteLLM handlers share one struct: the injected client factories, the logging handle and the error converter
pub struct VideoTransport<'deps, Source: ?Sized, Log: ?Sized, Convert: ?Sized> {
    pub clients: &'deps Source,
    pub logging: &'deps Log,
    pub converter: &'deps Convert,
}

// The call fields the shared front half reads
#[derive(Debug, Clone, Copy)]
pub(crate) struct PrepareFields<'call> {
    pub(crate) provider: &'call str,
    pub(crate) litellm_params: &'call Params,
    pub(crate) extra_headers: Option<&'call Headers>,
    pub(crate) api_key: Option<&'call SecretString>,
}

// The call's client (supplied or built) with the environment headers and the completed URL
pub(crate) struct Prepared<'call, Client> {
    pub(crate) client: Selected<'call, Client>,
    pub(crate) headers: Headers,
    pub(crate) api_base: String,
}

impl<Client> Prepared<'_, Client> {
    // the last three arguments of every video request transform
    pub(crate) const fn context<'borrow>(
        &'borrow self,
        litellm_params: &'borrow Params,
    ) -> VideoRequestContext<'borrow> {
        VideoRequestContext {
            api_base: &self.api_base,
            litellm_params,
            headers: &self.headers,
        }
    }
}

impl<'deps, Source, Log, Convert> VideoTransport<'deps, Source, Log, Convert>
where
    Source: ?Sized,
    Log: ?Sized,
    Convert: ?Sized,
{
    #[must_use]
    pub const fn new(
        clients: &'deps Source,
        logging: &'deps Log,
        converter: &'deps Convert,
    ) -> Self {
        Self {
            clients,
            logging,
            converter,
        }
    }
}

impl<Source, Log, Convert> VideoTransport<'_, Source, Log, Convert>
where
    Source: VideoClientSource + ?Sized,
    Log: VideoLoggingHandle + ?Sized,
    Convert: VideoErrorConvert + ?Sized,
{
    fn created_client(
        &self,
        params: &VideoParams,
        provider: &str,
        mode: ClientKind,
    ) -> Result<Source::Client, VideoClientError> {
        match mode {
            ClientKind::Sync => self.clients.sync_client(params.ssl_verify),
            ClientKind::Asynchronous => self.clients.async_client(provider, params.ssl_verify),
        }
    }

    // isinstance(client, HTTPHandler): a supplied client is used only when its mode is the one this path runs
    fn select<'call>(
        &self,
        supplied: Option<&SuppliedClient<'call, Source::Client>>,
        params: &VideoParams,
        provider: &str,
        mode: ClientKind,
    ) -> Result<Selected<'call, Source::Client>, VideoClientError> {
        if let Some(client) = supplied
            && client.kind == mode
        {
            return Ok(Selected::Supplied(client.transport));
        }
        self.created_client(params, provider, mode)
            .map(Selected::Created)
    }

    // The front half of all four handlers: the client, validate_environment and get_complete_url, each outside every try
    pub(crate) fn prepare<'call>(
        &self,
        supplied: Option<&SuppliedClient<'call, Source::Client>>,
        fields: PrepareFields<'call>,
        config: &dyn BaseVideoConfig,
        mode: ClientKind,
    ) -> Result<Prepared<'call, Source::Client>, VideoEditTransportError> {
        let params = VideoParams::read(fields.litellm_params)?;
        let client = self
            .select(supplied, &params, fields.provider, mode)
            .map_err(VideoEditTransportError::Client)?;
        let empty = Headers::new();
        let extra = fields.extra_headers.unwrap_or(&empty);
        let mut headers = config.validate_environment(VideoEnvironment {
            headers: extra,
            model: "",
            api_key: fields.api_key.or(params.api_key.as_ref()),
            litellm_params: Some(fields.litellm_params),
        });
        // LiteLLM updates the validated headers with extra_headers a second time, so the caller's values win
        if let Some(extra_headers) = fields.extra_headers {
            headers.extend(
                extra_headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        let api_base = config
            .get_complete_url("", params.api_base.as_deref(), fields.litellm_params)
            .map_err(VideoEditTransportError::Config)?;
        Ok(Prepared {
            client,
            headers,
            api_base,
        })
    }

    // _handle_error's second half: the shared converter picks the status, text and headers, and the provider names the class
    pub(crate) fn provider_error(
        &self,
        config: &dyn BaseVideoConfig,
        facts: &FailureFacts,
    ) -> VideoEditTransportError {
        let exception = self.converter.convert(facts);
        let mut error = config.get_error_class(
            &exception.message,
            exception.status_code,
            &header_pairs(&exception.headers),
        );
        // _handle_error only ever marks the flag, so a provider's own false stays as it is
        if exception.status_code_is_synthesized {
            error.status_code_is_synthesized = true;
        }
        VideoEditTransportError::Provider(Box::new(error))
    }
}
