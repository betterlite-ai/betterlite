use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_openai::vector_stores::Headers as OpenAiHeaders;

use super::super::config::{Params, SearchConfig, SignRequest};
use super::super::error::{SearchError, SigningFailure};
use crate::api::vector_store_http_write::service::ClientKind;

pub type SignedSearch = (OpenAiHeaders, Option<Vec<u8>>);
pub type SignFuture<'seam> =
    Pin<Box<dyn Future<Output = Result<SignedSearch, SigningFailure>> + Send + 'seam>>;

// Everything sign_request reads, owned so an AWS-signed config can sign on a blocking thread
pub struct SignInput {
    pub headers: OpenAiHeaders,
    pub optional_params: Params,
    pub request_data: Params,
    pub api_base: String,
}

impl SignInput {
    #[must_use]
    pub fn request(&self) -> SignRequest<'_> {
        SignRequest {
            headers: &self.headers,
            optional_params: &self.optional_params,
            request_data: &self.request_data,
            api_base: &self.api_base,
        }
    }
}

// B595's sign_request_off_loop_if_aws: a SignsRequestsWithAWS config signs on the blocking pool, every other config inline
pub trait SigningSeam: Send + Sync {
    fn sign(&self, config: Arc<dyn SearchConfig>, request: SignInput) -> SignFuture<'_>;
}

// The sync handler calls sign_request inline; the async one hands it to the seam
pub(super) async fn sign<G>(
    config: Arc<dyn SearchConfig>,
    seam: &G,
    mode: ClientKind,
    request: SignInput,
) -> Result<SignedSearch, SearchError>
where
    G: SigningSeam + ?Sized,
{
    if mode == ClientKind::Sync {
        return Ok(config.sign_request(request.request()));
    }
    seam.sign(config, request)
        .await
        .map_err(SearchError::Signing)
}
