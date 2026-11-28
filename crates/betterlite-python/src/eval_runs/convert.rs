use std::mem;
use std::time::Duration;

use betterlite_core::api::eval_run_lifecycle::DEFAULT_PROVIDER;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;

mod tests;

// One run call's arguments before they are split into the parameters every run operation shares.
pub(crate) type Fields = IndexMap<String, Json>;

// A request argument outside the type LiteLLM declares: docs/compatibility.md answers it with a 400 naming the field.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the eval run payload is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {name} field does not have the declared type")]
    FieldType {
        provider: String,
        name: &'static str,
    },
}

pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}

pub(crate) fn field_error(provider: &str, name: &'static str) -> ConvertError {
    ConvertError::FieldType {
        provider: provider.to_owned(),
        name,
    }
}

// One declared field: absent or null takes its default, and any other value has to read as the field's type.
fn read<T>(
    fields: &mut Fields,
    name: &'static str,
    provider: &str,
) -> Result<Option<T>, ConvertError>
where
    T: DeserializeOwned,
{
    let Some(value) = fields
        .shift_remove(name)
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(None);
    };
    let plain = serde_json::to_value(value).map_err(|source| ConvertError::Json { source })?;
    serde_json::from_value(plain)
        .map(Some)
        .map_err(|_wrong| field_error(provider, name))
}

// One run payload's parse body, generated once: the shared parameters first, then its own fields in the order the errors name them.
macro_rules! parse_payload {
    ($payload:ident, required { $($required:ident),* $(,)? } optional { $($optional:ident),* $(,)? }) => {
        impl $payload {
            pub(crate) fn parse(text: &str) -> Result<Self, ConvertError> {
                let mut fields: Fields = parse(text)?;
                let params = Params::split(&mut fields)?;
                let provider = params.provider().to_owned();
                $(let $required = read(&mut fields, stringify!($required), &provider)?.ok_or_else(|| field_error(&provider, stringify!($required)))?;)*
                $(let $optional = read(&mut fields, stringify!($optional), &provider)?;)*
                Ok(Self {
                    $($required,)*
                    $($optional,)*
                    params: params.keywords(&mut fields),
                })
            }
        }
    };
}

// Python sends the caller's timeout in seconds; null leaves it unset and an explicit zero is sent as a zero deadline.
fn timeout(fields: &mut Fields, provider: &str) -> Result<Option<Duration>, ConvertError> {
    let Some(seconds) = read::<f64>(fields, "timeout", provider)? else {
        return Ok(None);
    };
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|_invalid| field_error(provider, "timeout"))
}

// The parameters every run operation declares, with the keywords the caller left over.
pub(crate) struct Params {
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_query: Option<Object>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

impl Params {
    // custom_llm_provider is read first, so a later field's 400 names the provider the caller chose.
    fn split(fields: &mut Fields) -> Result<Self, ConvertError> {
        let custom_llm_provider = read(fields, "custom_llm_provider", DEFAULT_PROVIDER)?;
        let provider = custom_llm_provider.as_deref().unwrap_or(DEFAULT_PROVIDER);
        let extra_headers = read(fields, "extra_headers", provider)?;
        let extra_query = read(fields, "extra_query", provider)?;
        let given_timeout = timeout(fields, provider)?;
        Ok(Self {
            extra_headers,
            extra_query,
            timeout: given_timeout,
            custom_llm_provider,
            kwargs: IndexMap::new(),
        })
    }

    // absent custom_llm_provider takes LiteLLM's own default, before the provider manager sees the name
    pub(crate) fn provider(&self) -> &str {
        self.custom_llm_provider
            .as_deref()
            .unwrap_or(DEFAULT_PROVIDER)
    }

    // every name left after the named arguments were read is one of the caller's **kwargs
    fn keywords(mut self, fields: &mut Fields) -> Self {
        self.kwargs = mem::take(fields);
        self
    }
}

// create_run's arguments: the id, the data source, and the name and metadata LiteLLM's body reads.
pub(crate) struct CreateRunPayload {
    pub(crate) eval_id: String,
    pub(crate) data_source: Object,
    pub(crate) name: Option<String>,
    pub(crate) metadata: Option<Object>,
    pub(crate) extra_body: Option<Object>,
    pub(crate) params: Params,
}

parse_payload!(
    CreateRunPayload,
    required { eval_id, data_source }
    optional { name, metadata, extra_body }
);

// list_runs' pagination arguments, each None when the caller did not pass it.
pub(crate) struct ListRunPayload {
    pub(crate) eval_id: String,
    pub(crate) limit: Option<i64>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) order: Option<String>,
    pub(crate) params: Params,
}

parse_payload!(
    ListRunPayload,
    required { eval_id }
    optional { limit, after, before, order }
);

// get_run, cancel_run and delete_run's arguments: both ids the path is built from and the shared parameters.
pub(crate) struct RunIdPayload {
    pub(crate) eval_id: String,
    pub(crate) run_id: String,
    pub(crate) params: Params,
}

parse_payload!(RunIdPayload, required { eval_id, run_id } optional {});
