use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_context::resource_config_selection::responses::ResponsesConfigTag;
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::ListInputItemsQuery;
use indexmap::IndexMap;

use crate::api::responses_read_http::config::{ReadConfig, ReadParams};
use crate::api::responses_read_http::error::ReadError;
use crate::api::responses_read_http::service::{
    ClientKind, InputItemsCall, LogContext, ReadCall, ReadDeps, ReadOutcome, ReadTarget,
    ReadTransport, RetrieveCall, prepare, send,
};
use crate::api::vector_store_http_write::service::ClientSource;

// the chosen config tag travels beside the B2416 arguments, so the selection is visible without a socket
pub struct ReadRequest<'call, 'state, 'env> {
    pub tag: &'call ResponsesConfigTag,
    pub response_id: &'call str,
    pub params: &'call ReadParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub query: Option<&'call ListInputItemsQuery>,
    pub timeout: Option<Duration>,
    pub mode: ClientKind,
    pub log: LogContext<'state, 'env>,
}

// the read seam: production runs B2416 over a pooled client, a test or a case can answer without one
pub trait ResponsesReadExecutor: Send + Sync {
    fn read(
        &self,
        request: ReadRequest<'_, '_, '_>,
    ) -> impl Future<Output = Result<ReadOutcome, ReadError>> + Send;
}

pub struct HttpReadExecutor<Config, Source> {
    pub config: Config,
    pub source: Source,
}

impl<Config, Source> ResponsesReadExecutor for HttpReadExecutor<Config, Source>
where
    Config: ReadConfig,
    Source: ClientSource<Client: ReadTransport>,
{
    async fn read(&self, request: ReadRequest<'_, '_, '_>) -> Result<ReadOutcome, ReadError> {
        let target = ReadTarget {
            response_id: request.response_id,
            params: request.params,
            extra_headers: request.extra_headers,
            timeout: request.timeout,
        };
        let call = match request.query {
            Some(query) => ReadCall::InputItems(InputItemsCall { target, query }),
            None => ReadCall::Retrieve(RetrieveCall {
                target,
                extra_body: request.extra_body,
            }),
        };
        let deps = ReadDeps {
            config: &self.config,
            source: &self.source,
            supplied: None,
        };
        let mut log = request.log;
        let prepared = prepare(&deps, request.mode, &call, &mut log)?;
        send(prepared).await
    }
}
