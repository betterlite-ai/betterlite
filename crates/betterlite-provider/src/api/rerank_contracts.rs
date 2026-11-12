use std::collections::BTreeMap;
use std::error::Error;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse,
};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Number;
use thiserror::Error;

pub type Params = BTreeMap<String, Json>;
pub type HeaderMap = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, String>;

// LiteLLM's ModelInfo prices a rerank query with input_cost_per_query and spells a missing price as null.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RerankModelInfo {
    #[serde(default)]
    pub input_cost_per_query: Option<ExactDecimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Number")]
pub struct ExactDecimal(pub Decimal);

impl TryFrom<Number> for ExactDecimal {
    type Error = DecimalLexemeError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        parse_decimal_lexeme(&number.to_string()).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RerankCostError {
    #[error("the query cost does not fit an exact decimal")]
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankProviderResponse {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RerankLogging {
    pub model_call_details: Params,
}

#[derive(Debug, Clone, Copy)]
pub struct EnvironmentRequest<'request> {
    pub headers: &'request Headers,
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub optional_params: Option<&'request Params>,
    pub litellm_params: Option<&'request Params>,
}

#[derive(Debug)]
pub struct ResponseTransform<'request> {
    pub model: &'request str,
    pub raw_response: &'request RerankProviderResponse,
    pub logging: &'request mut RerankLogging,
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request Params,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
}

// Python's `return_documents: bool | None = True` argument: Default is the argument the caller omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnDocuments {
    Default,
    Null,
    Value(bool),
}

#[derive(Debug)]
pub struct OptionalRerankParams<'request> {
    pub model: &'request str,
    pub drop_params: bool,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub custom_llm_provider: Option<&'request str>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'request str>,
    pub non_default_params: Option<&'request mut Params>,
}

#[derive(Debug)]
pub struct MapCohereRerankParams<'request> {
    pub non_default_params: &'request mut Params,
    pub model: &'request str,
    pub drop_params: bool,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub custom_llm_provider: Option<&'request str>,
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: Option<bool>,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<&'request str>,
}

pub trait BaseRerankConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn validate_environment(&self, request: EnvironmentRequest<'_>)
    -> Result<Headers, Self::Error>;

    // an empty Params is Params::default(), so the Ok(Default::default()) mutant is equivalent
    #[cfg_attr(test, mutants::skip)]
    fn transform_rerank_request(
        &self,
        _model: &str,
        _optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        Ok(Params::new())
    }

    fn transform_rerank_response(
        &self,
        model_response: RerankResponse,
        _request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        Ok(model_response)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        api_base.unwrap_or_default().to_owned()
    }

    fn supported_rerank_params(&self, model: &str) -> Vec<String>;

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: HeaderMap,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers,
        }
    }

    fn calculate_rerank_cost(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
        billed_units: Option<&RerankBilledUnits>,
        model_info: Option<&RerankModelInfo>,
    ) -> Result<(Decimal, Decimal), RerankCostError> {
        let Some(search_units) = billed_units.and_then(|units| units.search_units.flatten()) else {
            return Ok((Decimal::ZERO, Decimal::ZERO));
        };
        let Some(price) =
            model_info.and_then(|info| info.input_cost_per_query.map(|value| value.0))
        else {
            return Ok((Decimal::ZERO, Decimal::ZERO));
        };
        let prompt_cost = price
            .checked_mul(Decimal::from(search_units))
            .ok_or(RerankCostError::Overflow)?;
        Ok((prompt_cost, Decimal::ZERO))
    }
}

// Provider dispatch registers this module's request, result and prepared placeholders; the wire records live in betterlite-openai.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RerankRequest;
pub struct RerankResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RerankPrepared;

fn insert_optional_params(
    non_default_params: &mut Params,
    request: &OptionalRerankParams<'_>,
) -> Option<bool> {
    non_default_params.insert("query".to_owned(), Json::Text(request.query.to_owned()));
    if let Some(top_n) = request.top_n {
        non_default_params.insert("top_n".to_owned(), Json::Number(Number::from(top_n)));
    }
    non_default_params.insert(
        "documents".to_owned(),
        Json::List(
            request
                .documents
                .iter()
                .map(RerankDocument::as_json)
                .collect(),
        ),
    );
    let return_documents = match request.return_documents {
        ReturnDocuments::Default => Some(true),
        ReturnDocuments::Null => None,
        ReturnDocuments::Value(value) => Some(value),
    };
    if let Some(value) = return_documents {
        non_default_params.insert("return_documents".to_owned(), Json::Bool(value));
    }
    if let Some(value) = request.max_chunks_per_doc {
        non_default_params.insert(
            "max_chunks_per_doc".to_owned(),
            Json::Number(Number::from(value)),
        );
    }
    if let Some(value) = request.max_tokens_per_doc {
        non_default_params.insert(
            "max_tokens_per_doc".to_owned(),
            Json::Number(Number::from(value)),
        );
    }
    if let Some(value) = request.instruction {
        non_default_params.insert("instruction".to_owned(), Json::Text(value.to_owned()));
    }
    return_documents
}

pub fn get_optional_rerank_params<Config>(
    config: &Config,
    mut request: OptionalRerankParams<'_>,
) -> Result<Params, Config::Error>
where
    Config: BaseRerankConfig + ?Sized,
{
    let mut allocated = Params::new();
    let non_default_params = match request.non_default_params.take() {
        Some(params) if !params.is_empty() => params,
        _ => &mut allocated,
    };
    let return_documents = insert_optional_params(non_default_params, &request);
    config.map_cohere_rerank_params(MapCohereRerankParams {
        non_default_params,
        model: request.model,
        drop_params: request.drop_params,
        query: request.query,
        documents: request.documents,
        custom_llm_provider: request.custom_llm_provider,
        top_n: request.top_n,
        rank_fields: request.rank_fields,
        return_documents,
        max_chunks_per_doc: request.max_chunks_per_doc,
        max_tokens_per_doc: request.max_tokens_per_doc,
        instruction: request.instruction,
    })
}

mod tests;
