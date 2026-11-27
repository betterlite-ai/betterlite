use std::time::Duration;

use betterlite_core::api::eval_create_list::DEFAULT_EVALS_PROVIDER;
use betterlite_core::api::eval_create_list::call::{
    CreateEvalCall, EvalsCallParams, ListEvalsCall,
};
use betterlite_core::api::eval_get_update::call::{GetEvalCall, UpdateEvalCall};
use betterlite_core::api::eval_lifecycle::EvalLifecycleCall;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{DataSourceConfig, EvalOrder, EvalOrderBy, GraderConfig};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;

use super::convert::{ConvertError, Fields, parse_fields, required, take, timeout};

// The arguments every eval operation declares, with the values the call's keywords carry.
pub(crate) struct Params {
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_query: IndexMap<String, Json>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

impl Params {
    pub(crate) fn call(&self) -> EvalsCallParams<'_> {
        EvalsCallParams {
            extra_headers: self.extra_headers.as_ref(),
            extra_query: &self.extra_query,
            timeout: self.timeout,
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            kwargs: &self.kwargs,
        }
    }

    // absent custom_llm_provider takes LiteLLM's own default, before the provider manager sees the name
    pub(crate) fn provider(&self) -> &str {
        self.custom_llm_provider
            .as_deref()
            .unwrap_or(DEFAULT_EVALS_PROVIDER)
    }
}

impl Params {
    // The shared parameters come out first; custom_llm_provider is read before the typed fields so their errors name it.
    fn from_arguments(arguments: &mut Fields) -> Result<Self, ConvertError> {
        let custom_llm_provider = take(arguments, "custom_llm_provider", DEFAULT_EVALS_PROVIDER)?;
        let provider = custom_llm_provider
            .as_deref()
            .unwrap_or(DEFAULT_EVALS_PROVIDER);
        Ok(Self {
            extra_headers: take(arguments, "extra_headers", provider)?,
            extra_query: take(arguments, "extra_query", provider)?.unwrap_or_default(),
            timeout: timeout(arguments, provider)?,
            custom_llm_provider,
            kwargs: IndexMap::new(),
        })
    }

    // every name left after the named arguments were read is one of the caller's **kwargs
    fn take_keywords(mut self, arguments: &mut Fields) -> Self {
        self.kwargs = std::mem::take(arguments);
        self
    }
}

// create_eval's arguments: the two required fields, the name, and the metadata LiteLLM accepts and never forwards.
pub(crate) struct CreatePayload {
    data_source_config: DataSourceConfig,
    testing_criteria: Vec<GraderConfig>,
    name: Option<String>,
    metadata: Option<IndexMap<String, Json>>,
    extra_body: IndexMap<String, Json>,
    pub(crate) params: Params,
}

impl CreatePayload {
    pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
        let mut arguments = parse_fields(text)?;
        let params = Params::from_arguments(&mut arguments)?;
        let provider = params.provider().to_owned();
        let data_source_config = required(&mut arguments, "data_source_config", &provider)?;
        let testing_criteria = required(&mut arguments, "testing_criteria", &provider)?;
        let name = take(&mut arguments, "name", &provider)?;
        let metadata = take(&mut arguments, "metadata", &provider)?;
        let extra_body = take(&mut arguments, "extra_body", &provider)?.unwrap_or_default();
        Ok(Self {
            data_source_config,
            testing_criteria,
            name,
            metadata,
            extra_body,
            params: params.take_keywords(&mut arguments),
        })
    }

    pub(crate) fn call(&self) -> CreateEvalCall<'_> {
        CreateEvalCall {
            data_source_config: self.data_source_config.clone(),
            testing_criteria: self.testing_criteria.clone(),
            name: self.name.as_deref(),
            metadata: self.metadata.as_ref(),
            extra_body: &self.extra_body,
            params: self.params.call(),
        }
    }
}

// list_evals' pagination arguments, each None when the caller did not pass it.
pub(crate) struct ListPayload {
    limit: Option<i64>,
    after: Option<String>,
    before: Option<String>,
    order: Option<EvalOrder>,
    order_by: Option<EvalOrderBy>,
    pub(crate) params: Params,
}

impl ListPayload {
    pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
        let mut arguments = parse_fields(text)?;
        let params = Params::from_arguments(&mut arguments)?;
        let provider = params.provider().to_owned();
        let limit = take(&mut arguments, "limit", &provider)?;
        let after = take(&mut arguments, "after", &provider)?;
        let before = take(&mut arguments, "before", &provider)?;
        let order = take(&mut arguments, "order", &provider)?;
        let order_by = take(&mut arguments, "order_by", &provider)?;
        Ok(Self {
            limit,
            after,
            before,
            order,
            order_by,
            params: params.take_keywords(&mut arguments),
        })
    }

    pub(crate) fn call(&self) -> ListEvalsCall<'_> {
        ListEvalsCall {
            limit: self.limit,
            after: self.after.as_deref(),
            before: self.before.as_deref(),
            order: self.order,
            order_by: self.order_by,
            params: self.params.call(),
        }
    }
}

// get_eval's arguments: the id its path is built from and the shared call parameters.
pub(crate) struct GetPayload {
    eval_id: String,
    pub(crate) params: Params,
}

impl GetPayload {
    pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
        let mut arguments = parse_fields(text)?;
        let params = Params::from_arguments(&mut arguments)?;
        let provider = params.provider().to_owned();
        let eval_id = required(&mut arguments, "eval_id", &provider)?;
        Ok(Self {
            eval_id,
            params: params.take_keywords(&mut arguments),
        })
    }

    pub(crate) fn call(&self) -> GetEvalCall<'_> {
        GetEvalCall {
            eval_id: &self.eval_id,
            params: self.params.call(),
        }
    }
}

// update_eval's arguments: the id, the two typed request fields and extra_body over the top.
pub(crate) struct UpdatePayload {
    eval_id: String,
    name: Option<String>,
    metadata: Option<IndexMap<String, Json>>,
    extra_body: IndexMap<String, Json>,
    pub(crate) params: Params,
}

impl UpdatePayload {
    pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
        let mut arguments = parse_fields(text)?;
        let params = Params::from_arguments(&mut arguments)?;
        let provider = params.provider().to_owned();
        let eval_id = required(&mut arguments, "eval_id", &provider)?;
        let name = take(&mut arguments, "name", &provider)?;
        let metadata = take(&mut arguments, "metadata", &provider)?;
        let extra_body = take(&mut arguments, "extra_body", &provider)?.unwrap_or_default();
        Ok(Self {
            eval_id,
            name,
            metadata,
            extra_body,
            params: params.take_keywords(&mut arguments),
        })
    }

    pub(crate) fn call(&self) -> UpdateEvalCall<'_> {
        UpdateEvalCall {
            eval_id: &self.eval_id,
            name: self.name.as_deref(),
            metadata: self.metadata.as_ref(),
            extra_body: &self.extra_body,
            params: self.params.call(),
        }
    }
}

// delete_eval and cancel_eval's arguments; extra_query is accepted and never read, as in LiteLLM.
pub(crate) struct LifecyclePayload {
    eval_id: String,
    pub(crate) params: Params,
}

impl LifecyclePayload {
    pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
        let mut arguments = parse_fields(text)?;
        let params = Params::from_arguments(&mut arguments)?;
        let provider = params.provider().to_owned();
        let eval_id = required(&mut arguments, "eval_id", &provider)?;
        Ok(Self {
            eval_id,
            params: params.take_keywords(&mut arguments),
        })
    }

    pub(crate) fn call(&self) -> EvalLifecycleCall<'_> {
        EvalLifecycleCall {
            eval_id: &self.eval_id,
            extra_headers: self.params.extra_headers.as_ref(),
            extra_query: None,
            timeout: self.params.timeout,
            custom_llm_provider: self.params.custom_llm_provider.as_deref(),
            kwargs: &self.params.kwargs,
        }
    }
}
