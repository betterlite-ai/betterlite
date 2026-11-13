use std::time::Duration;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use crate::api::batch_records::{BatchRecordsError, CreateBatchRequest, WireDecodeLimits};
use crate::batches::records::BatchObject;
use crate::files::records::BinaryContent;

use super::error::XaiBatchesError;
use super::jsonl::results_to_openai_jsonl;
use super::records::{XaiBatch, XaiBatchList, XaiBatchResultsPage, decode_xai_batch};
use super::transformation::{
    DEFAULT_BATCH_ENDPOINT, JSONL_CONTENT_TYPE, OpenAiBatchListResponse, create_endpoint,
    get_xai_auth_headers, raise_for_xai_status, results_params, to_create_batch_body,
    to_litellm_batch, to_openai_batch_list, xai_batches_url,
};
use super::transport::XaiBatchTransport;

type BatchResult = Result<BatchObject, XaiBatchesError>;

#[derive(Debug, Clone, Copy)]
pub struct CallContext<'call> {
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Duration,
}

pub struct XaiBatches<T> {
    pub transport: T,
    pub settings: Settings,
    pub limits: WireDecodeLimits,
}

type Target<'request> = (Method, Option<&'request str>, &'static str);

fn bounded_append(body: &mut Vec<u8>, encoded: &[u8], limit: usize) -> Result<(), XaiBatchesError> {
    let total = body
        .len()
        .checked_add(encoded.len())
        .ok_or_else(|| XaiBatchesError::Limit(BatchRecordsError::JsonTooLarge { limit }))?;
    if total > limit {
        return Err(XaiBatchesError::Limit(BatchRecordsError::JsonTooLarge {
            limit,
        }));
    }
    body.extend_from_slice(encoded);
    Ok(())
}

impl<T: XaiBatchTransport> XaiBatches<T> {
    // One async path for every verb: URL, credentials, body, then send, as the source orders them
    async fn call(
        &self,
        target: Target<'_>,
        query: Vec<(String, String)>,
        body: Option<&CreateBatchRequest>,
        context: CallContext<'_>,
    ) -> Result<UpstreamResponse, XaiBatchesError> {
        let (method, batch_id, suffix) = target;
        let base = xai_batches_url(context.api_base, batch_id, suffix, &self.settings)?;
        let headers = get_xai_auth_headers(HeaderMap::new(), context.api_key, &self.settings)?;
        let payload = match body {
            Some(data) => {
                let bytes = serde_json::to_vec(&to_create_batch_body(data)?)
                    .map_err(|source| XaiBatchesError::Encode { source })?;
                RequestBody::Json(bytes)
            }
            None => RequestBody::Empty,
        };
        let url = Url::parse(&base).map_err(|source| XaiBatchesError::BaseUrl { source })?;
        let response = self
            .transport
            .send(OutboundRequest {
                method,
                url,
                query,
                headers,
                body: payload,
                deadline: context.timeout,
            })
            .await
            .map_err(|source| XaiBatchesError::Transport { source })?;
        raise_for_xai_status(&response)?;
        Ok(response)
    }

    async fn batch_object(
        &self,
        target: Target<'_>,
        body: Option<&CreateBatchRequest>,
        endpoint: &str,
        context: CallContext<'_>,
    ) -> BatchResult {
        let response = self.call(target, Vec::new(), body, context).await?;
        let batch: XaiBatch = decode_xai_batch(&response.body, &self.limits)?;
        Ok(to_litellm_batch(&batch, endpoint))
    }

    pub async fn create_batch(
        &self,
        data: &CreateBatchRequest,
        context: CallContext<'_>,
    ) -> BatchResult {
        let target = (Method::POST, None, "");
        self.batch_object(target, Some(data), create_endpoint(data), context)
            .await
    }

    pub async fn retrieve_batch(&self, batch_id: &str, context: CallContext<'_>) -> BatchResult {
        let target = (Method::GET, Some(batch_id), "");
        self.batch_object(target, None, DEFAULT_BATCH_ENDPOINT, context)
            .await
    }

    pub async fn cancel_batch(&self, batch_id: &str, context: CallContext<'_>) -> BatchResult {
        let target = (Method::POST, Some(batch_id), ":cancel");
        self.batch_object(target, None, DEFAULT_BATCH_ENDPOINT, context)
            .await
    }

    pub async fn list_batches(
        &self,
        context: CallContext<'_>,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> Result<OpenAiBatchListResponse, XaiBatchesError> {
        let response = self
            .call(
                (Method::GET, None, ""),
                results_params(after, limit),
                None,
                context,
            )
            .await?;
        let page: XaiBatchList = decode_xai_batch(&response.body, &self.limits)?;
        Ok(to_openai_batch_list(&page))
    }

    // Pages are collected one at a time, so the accumulated bytes never grow past the caller's limit
    pub async fn batch_results_content(
        &self,
        batch_id: &str,
        context: CallContext<'_>,
    ) -> Result<BinaryContent, XaiBatchesError> {
        let mut body: Vec<u8> = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let response = self
                .call(
                    (Method::GET, Some(batch_id), "/results"),
                    results_params(after.as_deref(), None),
                    None,
                    context,
                )
                .await?;
            let page: XaiBatchResultsPage = decode_xai_batch(&response.body, &self.limits)?;
            let encoded = results_to_openai_jsonl(&page.results)?;
            bounded_append(&mut body, &encoded, self.limits.max_json_bytes)?;
            let Some(token) = page.pagination_token.filter(|token| !token.is_empty()) else {
                break;
            };
            if page.results.is_empty() {
                break;
            }
            after = Some(token);
        }
        Ok(BinaryContent {
            status: 200,
            headers: IndexMap::from([("content-type".to_owned(), JSONL_CONTENT_TYPE.to_owned())]),
            body,
        })
    }
}
