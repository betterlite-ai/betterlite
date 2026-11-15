use crate::google_count_tokens::decode::GoogleCountResponse;
use crate::google_token_counter::counter::CountTokensCall;
use crate::google_token_counter::error::CountTokensError;
use crate::partner_count::response::PartnerCount;

// LiteLLM's TokenCountResponse; the Google wrappers raise instead of catching, so the error fields stay false and None
#[derive(Debug)]
pub struct TokenCountResponse {
    pub total_tokens: i64,
    pub request_model: String,
    pub model_used: String,
    pub tokenizer_type: String,
    pub original_response: Option<CountReply>,
    pub error: bool,
    pub error_message: Option<String>,
    pub status_code: Option<u16>,
}

#[derive(Debug)]
pub enum CountReply {
    // boxed to stay within 3x of Partner, the limit variant_size_differences enforces
    Google(Box<GoogleCountResponse>),
    Partner(PartnerCount),
}

impl TokenCountResponse {
    #[must_use]
    pub fn new(
        call: &CountTokensCall<'_>,
        total_tokens: i64,
        tokenizer_type: String,
        original_response: CountReply,
    ) -> Self {
        Self {
            total_tokens,
            request_model: call.request_model.to_owned(),
            model_used: call.model_to_use.to_owned(),
            tokenizer_type,
            original_response: Some(original_response),
            error: false,
            error_message: None,
            status_code: None,
        }
    }
}

pub(crate) fn counted(total: u64) -> Result<i64, CountTokensError> {
    match i64::try_from(total) {
        Ok(counted) => Ok(counted),
        Err(_overflow) => Err(CountTokensError::Overflow),
    }
}
