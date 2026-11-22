use super::call::{Composition, Credentials, Operation};
use super::error::AzureProviderError;
use betterlite_config::loader::EnvironmentLookup as _;
use betterlite_config::sections::azure_endpoints::AZURE_DEFAULT_API_VERSION;
use betterlite_context::chat_config_selection::azure::is_o_series_model;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
pub(crate) const DEFAULT_API_TYPE: &str = "azure";
pub(crate) struct Resolved {
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_type: String,
    // the token the SDK authenticates with: only the call's own azure_ad_token parameter reaches it
    pub(crate) auth_token: Option<SecretString>,
    // main.py's resolved token: extra_body.azure_ad_token, then AZURE_AD_TOKEN
    pub(crate) azure_ad_token: Option<SecretString>,
    // the call's own headers and extra headers, and litellm.headers, each kept for its own merge
    pub(crate) headers: Option<Headers>,
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) globals_headers: Option<Headers>,
    pub(crate) max_retries: Option<Json>,
    pub(crate) extra_body: Object,
    pub(crate) default_api_version: String,
    pub(crate) o_series: bool,
}
pub(crate) fn convert<A, B>(value: &A) -> Result<B, AzureProviderError>
where
    A: Serialize + ?Sized,
    B: DeserializeOwned,
{
    let text =
        serde_json::to_string(value).map_err(|source| AzureProviderError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| AzureProviderError::Encode { source })
}
fn filled_secret(secret: Option<&SecretString>) -> Option<SecretString> {
    secret
        .filter(|text| !text.expose_secret().is_empty())
        .cloned()
}
fn api_type(composition: &Composition) -> String {
    composition
        .environment
        .lookup("AZURE_API_TYPE")
        .filter(|kind| !kind.is_empty())
        .unwrap_or_else(|| DEFAULT_API_TYPE.to_owned())
}
fn default_version(composition: &Composition) -> String {
    composition
        .globals
        .azure_default_api_version
        .clone()
        .unwrap_or_else(|| AZURE_DEFAULT_API_VERSION.to_owned())
}
fn take_extra_body(
    credentials: &mut Credentials,
) -> Result<(Object, Option<String>), AzureProviderError> {
    let Some(value) = credentials.extra_body.take() else {
        return Ok((Object::new(), None));
    };
    let Json::Object(mut fields) = value else {
        return Err(AzureProviderError::ExtraBodyNotAnObject);
    };
    let token = match fields.shift_remove("azure_ad_token") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text).filter(|text| !text.is_empty()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AzureProviderError::ExtraBodyToken);
        }
    };
    Ok((fields, token))
}
// embeddings hand kwargs.extra_body straight to the SDK, which merges it into the request body
fn embedding_extra_body(credentials: &mut Credentials) -> Result<Object, AzureProviderError> {
    match credentials.extra_body.take() {
        None | Some(Json::Null) => Ok(Object::new()),
        Some(Json::Object(fields)) => Ok(fields),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(AzureProviderError::ExtraBodyNotAnObject)
        }
    }
}
fn ad_token(composition: &Composition, from_body: Option<String>) -> Option<SecretString> {
    from_body
        .map(SecretString::from)
        .or_else(|| composition.secret("AZURE_AD_TOKEN"))
}
struct Ranks<'composition, 'credentials> {
    composition: &'composition Composition,
    credentials: &'credentials Credentials,
    operation: Operation,
}
impl Ranks<'_, '_> {
    // an explicit value, then the module globals, then the environment, where an empty base or version is still set
    fn ranked(&self, explicit: Option<&str>, global: Option<&str>, env: &str) -> Option<String> {
        explicit
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .or_else(|| global.filter(|text| !text.is_empty()).map(str::to_owned))
            .or_else(|| self.composition.environment.lookup(env))
    }
    fn api_base(&self) -> Option<String> {
        self.ranked(
            self.credentials.api_base.as_deref(),
            self.composition.globals.api_base.as_deref(),
            "AZURE_API_BASE",
        )
    }
    fn api_version(&self) -> Option<String> {
        self.ranked(
            self.credentials.api_version.as_deref(),
            self.composition.globals.api_version.as_deref(),
            "AZURE_API_VERSION",
        )
    }
    fn api_key(&self) -> Option<SecretString> {
        let globals = &self.composition.globals;
        let key = filled_secret(self.credentials.api_key.as_ref())
            .or_else(|| filled_secret(globals.api_key.as_ref()))
            .or_else(|| filled_secret(globals.azure_key.as_ref()));
        if self.operation == Operation::Embedding {
            return key.or_else(|| self.composition.secret("AZURE_API_KEY"));
        }
        key.or_else(|| self.composition.secret("AZURE_OPENAI_API_KEY"))
            .or_else(|| self.composition.secret("AZURE_API_KEY"))
    }
    // the text and embedding branches own a missing-base failure; chat lets the URL builder refuse it
    fn bases(
        &self,
        default_api_version: &str,
    ) -> Result<(Option<String>, Option<String>), AzureProviderError> {
        let api_base = self.api_base();
        let api_version = self.api_version();
        match self.operation {
            Operation::Text => Ok((
                Some(api_base.ok_or(AzureProviderError::MissingTextApiBase)?),
                api_version,
            )),
            Operation::Embedding => Ok((
                Some(api_base.ok_or(AzureProviderError::MissingEmbeddingApiBase)?),
                Some(defaulted(api_version, default_api_version)),
            )),
            Operation::Chat => Ok((api_base, Some(defaulted(api_version, default_api_version)))),
        }
    }
}
// chat and embeddings end their version chain with the module default, so an empty one falls through
fn defaulted(api_version: Option<String>, default_api_version: &str) -> String {
    api_version
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| default_api_version.to_owned())
}
pub(crate) fn resolve(
    composition: &Composition,
    mut credentials: Credentials,
    operation: Operation,
    detection_model: Option<&str>,
) -> Result<Resolved, AzureProviderError> {
    let default_api_version = default_version(composition);
    let (extra_body, from_body) = match operation {
        Operation::Embedding => (embedding_extra_body(&mut credentials)?, None),
        Operation::Chat | Operation::Text => take_extra_body(&mut credentials)?,
    };
    let ranks = Ranks {
        composition,
        credentials: &credentials,
        operation,
    };
    let (api_base, api_version) = ranks.bases(&default_api_version)?;
    let max_retries = (operation == Operation::Chat)
        .then(|| credentials.max_retries.clone())
        .flatten();
    Ok(Resolved {
        api_base,
        api_version,
        api_key: ranks.api_key(),
        api_type: api_type(composition),
        auth_token: filled_secret(credentials.azure_ad_token.as_ref()),
        azure_ad_token: ad_token(composition, from_body),
        headers: credentials.headers.clone(),
        extra_headers: credentials.extra_headers.clone(),
        globals_headers: composition
            .globals
            .headers
            .clone()
            .filter(|map| !map.is_empty()),
        max_retries,
        extra_body,
        default_api_version,
        o_series: detection_model.is_some_and(is_o_series_model),
    })
}
pub(crate) fn merge_defaults<T>(
    optional: &mut Object,
    defaults: &T,
) -> Result<(), AzureProviderError>
where
    T: Serialize + ?Sized,
{
    let values: Object = convert(defaults)?;
    for (name, value) in values {
        if value != Json::Null && !optional.contains_key(&name) {
            optional.insert(name, value);
        }
    }
    Ok(())
}
