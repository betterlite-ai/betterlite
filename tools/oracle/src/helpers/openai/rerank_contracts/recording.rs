use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params,
};
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error("the fixture mapper refused the parameters")]
    Refused,
}

#[derive(Default)]
pub(super) struct Recording {
    pub(super) calls: Mutex<Vec<Json>>,
}

impl BaseRerankConfig for Recording {
    type Error = FixtureError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        Ok(request.headers.clone())
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        if request.query == "refuse" {
            return Err(FixtureError::Refused);
        }
        let recorded = record(&request);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        Ok(Params::from([("mapped".to_owned(), Json::Bool(true))]))
    }
}

pub(super) fn object_of(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<IndexMap<_, _>>(),
    )
}

fn record(request: &MapCohereRerankParams<'_>) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        "non_default_params".to_owned(),
        object_of(request.non_default_params),
    );
    fields.insert("model".to_owned(), Json::Text(request.model.to_owned()));
    fields.insert("drop_params".to_owned(), Json::Bool(request.drop_params));
    fields.insert("query".to_owned(), Json::Text(request.query.to_owned()));
    fields.insert(
        "documents".to_owned(),
        Json::List(
            request
                .documents
                .iter()
                .map(RerankDocument::as_json)
                .collect(),
        ),
    );
    insert_optionals(&mut fields, request);
    Json::Object(fields)
}

fn insert_optionals(fields: &mut IndexMap<String, Json>, request: &MapCohereRerankParams<'_>) {
    fields.insert(
        "custom_llm_provider".to_owned(),
        request
            .custom_llm_provider
            .map_or(Json::Null, |provider| Json::Text(provider.to_owned())),
    );
    fields.insert(
        "top_n".to_owned(),
        request
            .top_n
            .map_or(Json::Null, |top_n| Json::Number(Number::from(top_n))),
    );
    fields.insert(
        "rank_fields".to_owned(),
        request.rank_fields.map_or(Json::Null, |names| {
            Json::List(names.iter().map(|name| Json::Text(name.clone())).collect())
        }),
    );
    fields.insert(
        "return_documents".to_owned(),
        request.return_documents.map_or(Json::Null, Json::Bool),
    );
    fields.insert(
        "max_chunks_per_doc".to_owned(),
        request
            .max_chunks_per_doc
            .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
    );
    fields.insert(
        "max_tokens_per_doc".to_owned(),
        request
            .max_tokens_per_doc
            .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
    );
    fields.insert(
        "instruction".to_owned(),
        request
            .instruction
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
}
