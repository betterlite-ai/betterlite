use std::time::Duration;

use betterlite_core::api::eval_create_list::DEFAULT_EVALS_PROVIDER;
use betterlite_core::api::eval_create_list::call::EvalsCallParams;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{DataSourceConfig, EvalOrder, EvalOrderBy, GraderConfig};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;

use super::convert::{ConvertError, Fields, field, parse, take, timeout};

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

// One payload's parse body, generated once: the shared parameters first, then its own fields in the order the errors name them.
macro_rules! parse_payload {
    ($payload:ident, required { $($required:ident),* $(,)? } optional { $($optional:ident),* $(,)? } default { $($default:ident),* $(,)? }) => {
        impl $payload {
            pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
                let mut arguments = parse::<Fields>(text)?;
                let params = Params::from_arguments(&mut arguments)?;
                let provider = params.provider().to_owned();
                $(let $required = take(&mut arguments, stringify!($required), &provider)?.ok_or_else(|| field(&provider, stringify!($required)))?;)*
                $(let $optional = take(&mut arguments, stringify!($optional), &provider)?;)*
                $(let $default = take(&mut arguments, stringify!($default), &provider)?.unwrap_or_default();)*
                Ok(Self {
                    $($required,)*
                    $($optional,)*
                    $($default,)*
                    params: params.take_keywords(&mut arguments),
                })
            }
        }
    };
}

// create_eval's arguments: the two required fields, the name, and the metadata LiteLLM accepts and never forwards.
pub(crate) struct CreatePayload {
    pub(crate) data_source_config: DataSourceConfig,
    pub(crate) testing_criteria: Vec<GraderConfig>,
    pub(crate) name: Option<String>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: IndexMap<String, Json>,
    pub(crate) params: Params,
}

parse_payload!(
    CreatePayload,
    required { data_source_config, testing_criteria }
    optional { name, metadata }
    default { extra_body }
);

// list_evals' pagination arguments, each None when the caller did not pass it.
pub(crate) struct ListPayload {
    pub(crate) limit: Option<i64>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) order: Option<EvalOrder>,
    pub(crate) order_by: Option<EvalOrderBy>,
    pub(crate) params: Params,
}

parse_payload!(
    ListPayload,
    required {}
    optional { limit, after, before, order, order_by }
    default {}
);

// update_eval's arguments: the id, the two typed request fields and extra_body over the top.
pub(crate) struct UpdatePayload {
    pub(crate) eval_id: String,
    pub(crate) name: Option<String>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    pub(crate) extra_body: IndexMap<String, Json>,
    pub(crate) params: Params,
}

parse_payload!(
    UpdatePayload,
    required { eval_id }
    optional { name, metadata }
    default { extra_body }
);

// get_eval, delete_eval and cancel_eval's arguments: the id the path is built from and the shared call parameters.
pub(crate) struct IdPayload {
    pub(crate) eval_id: String,
    pub(crate) params: Params,
}

parse_payload!(IdPayload, required { eval_id } optional {} default {});
