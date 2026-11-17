use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, ClientSecretRecord, CredentialSources, OidcExchangeInput,
    ProviderFailure, SourceFailure, TokenReply, UsernamePasswordRecord,
};
use betterlite_config::sections::foundry_credentials::Settings as FoundrySettings;
use betterlite_config::sections::foundry_image_edits::Settings as VersionSettings;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::model_info::ModelInfoAccessors;
use crate::mai_images::reply::{ImageLogging, PostCall};

pub(super) const FLUX1: &str = "FLUX-1-kontext-pro";
pub(super) const FLUX2: &str = "FLUX.2-pro";
pub(super) const FLEX: &str = "FLUX-2-flex";
pub(super) const MAI: &str = "MAI-Image-2.5";
pub(super) const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfirst";
pub(super) const JPEG: &[u8] = b"\xff\xd8\xff\xe0second";

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn params(text: &str) -> ImageParams {
    serde_json::from_str(text).expect("a JSON object")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn encoded(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(super) fn foundry_settings(base: Option<&str>, key: Option<&str>) -> FoundrySettings {
    FoundrySettings {
        api_base: base.map(str::to_owned),
        api_key: key.map(secret),
    }
}

pub(super) fn version_settings(version: Option<&str>) -> VersionSettings {
    VersionSettings {
        api_version: version.map(str::to_owned),
    }
}

pub(super) fn accessors<'data>(
    environment: &'data FoundrySettings,
    global_api_base: Option<&'data str>,
    global_api_key: Option<&'data SecretString>,
) -> ModelInfoAccessors<'data> {
    ModelInfoAccessors {
        global_api_base,
        global_api_key,
        environment_api_base: environment.api_base.as_deref(),
        environment_api_key: environment.api_key.as_ref(),
    }
}

// no probe in the image edit cases reaches a grant, so every source refuses instead of making a call
pub(super) struct NoSources;

fn refusal() -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from("this fixture resolves no credential sources"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refusal())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refusal())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refusal())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(refusal())
    }
}

pub(super) const ENTRA_TOKEN: &str = "entra-fixture-token";

pub(super) struct TokenSource;

impl CallableTokenProvider for TokenSource {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from(ENTRA_TOKEN)))
    }
}

// every grant answers with the same fixture token, so the header proves the Entra path ran
pub(super) struct EntraSources;

impl CredentialSources for EntraSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Ok(Arc::new(TokenSource))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Ok(Arc::new(TokenSource))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Ok(Arc::new(TokenSource))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        Some(Arc::new(TokenSource))
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Ok(SecretString::from(ENTRA_TOKEN))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PostCallRecord {
    pub(super) input: String,
    pub(super) api_key: Option<String>,
    pub(super) complete_input_dict: Object,
    pub(super) original_response: Object,
}

// the MAI edit transform logs exactly once, so one slot records the call without a collection to keep in order
#[derive(Default)]
pub(super) struct Logger {
    pub(super) recorded: Option<PostCallRecord>,
}

impl ImageLogging for Logger {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.recorded = Some(PostCallRecord {
            input: call.input.to_owned(),
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            complete_input_dict: call.complete_input_dict.clone(),
            original_response: call.original_response.clone(),
        });
    }
}
