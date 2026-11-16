use std::collections::BTreeMap;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_config::sections::foundry_credentials::Settings as FoundrySettings;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::credentials::error::CredentialsError;
use crate::credentials::model_info::ModelInfoAccessors;
use crate::image_edits::auth::{ValidateEnvironmentRequest, validate_environment};
use crate::image_edits::error::ImageEditError;

use super::support::{self, EntraSources, FLUX1, FLUX2, MAI, NoSources};

const BASE: &str = "https://res.services.ai.azure.com";

struct Call {
    model: &'static str,
    key: Option<SecretString>,
    global_key: Option<SecretString>,
    params: Option<Object>,
    environment: FoundrySettings,
    environment_vars: BTreeMap<String, String>,
    headers: Headers,
}

impl Call {
    fn new(model: &'static str) -> Self {
        Self {
            model,
            key: None,
            global_key: None,
            params: None,
            environment: support::foundry_settings(Some(BASE), None),
            environment_vars: BTreeMap::new(),
            headers: Headers::new(),
        }
    }
}

fn validate(call: &Call, sources: &dyn CredentialSources) -> Result<Headers, ImageEditError> {
    validate_environment(ValidateEnvironmentRequest {
        headers: call.headers.clone(),
        model: call.model,
        api_key: call.key.as_ref(),
        params: call.params.as_ref(),
        environment: &call.environment_vars,
        accessors: ModelInfoAccessors {
            global_api_base: None,
            global_api_key: call.global_key.as_ref(),
            environment_api_base: call.environment.api_base.as_deref(),
            environment_api_key: call.environment.api_key.as_ref(),
        },
        refresh_enabled: false,
        sources,
    })
}

pub(super) fn foundry_edit_auth() {
    key_headers();
    entra_fallback();
    key_precedence();
    missing_key();
    base_resolution();
}

fn key_headers() {
    let caller = support::headers(&[("X-Trace", "1"), ("Content-Type", "multipart/form-data")]);
    let mut flux1 = Call::new(FLUX1);
    flux1.key = Some(support::secret("flux-key"));
    flux1.headers = caller.clone();
    assert_eq!(
        validate(&flux1, &NoSources).unwrap(),
        support::headers(&[
            ("Api-Key", "flux-key"),
            ("X-Trace", "1"),
            ("Content-Type", "multipart/form-data"),
        ])
    );

    let mut flux2 = Call::new(FLUX2);
    flux2.key = Some(support::secret("flux-key"));
    flux2.headers = caller.clone();
    assert_eq!(
        validate(&flux2, &NoSources).unwrap(),
        support::headers(&[
            ("Api-Key", "flux-key"),
            ("X-Trace", "1"),
            ("Content-Type", "application/json"),
        ])
    );

    let mut mai = Call::new(MAI);
    mai.key = Some(support::secret("mai-key"));
    mai.headers = caller;
    assert_eq!(
        validate(&mai, &NoSources).unwrap(),
        support::headers(&[
            ("api-key", "mai-key"),
            ("X-Trace", "1"),
            ("Content-Type", "multipart/form-data"),
        ])
    );
}

fn entra_fallback() {
    let params = support::object(
        r#"{"tenant_id": "tenant", "client_id": "client", "client_secret": "grant"}"#,
    );
    for (model, header) in [(FLUX1, "Api-Key"), (FLUX2, "Api-Key"), (MAI, "api-key")] {
        let mut delegated = Call::new(model);
        delegated.params = Some(params.clone());
        let headers = validate(&delegated, &EntraSources).unwrap();
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some(concat!("Bearer ", "entra-fixture-token"))
        );
        assert_eq!(headers.get(header), None, "{model}");
        assert_eq!(
            headers.get("Content-Type").map(String::as_str),
            (model == FLUX2).then_some("application/json"),
            "{model}"
        );
    }
}

fn key_precedence() {
    let mut call = Call::new(FLUX1);
    call.key = Some(support::secret("explicit"));
    call.global_key = Some(support::secret("global"));
    call.environment = support::foundry_settings(None, Some("environment"));
    assert_eq!(
        validate(&call, &NoSources).unwrap().get("Api-Key"),
        Some(&String::from("explicit"))
    );

    call.key = None;
    assert_eq!(
        validate(&call, &NoSources).unwrap().get("Api-Key"),
        Some(&String::from("global"))
    );

    call.global_key = None;
    assert_eq!(
        validate(&call, &NoSources).unwrap().get("Api-Key"),
        Some(&String::from("environment"))
    );
}

fn missing_key() {
    let error = validate(&Call::new(FLUX1), &NoSources).unwrap_err();
    match &error {
        ImageEditError::Credentials(CredentialsError::MissingCredentials { env_var }) => {
            assert_eq!(env_var, "AZURE_AI_API_KEY");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(error.status_code(), None);
}

fn base_resolution() {
    let environment = support::foundry_settings(Some(BASE), None);
    let accessors = ModelInfoAccessors {
        global_api_base: Some("https://global.test"),
        global_api_key: None,
        environment_api_base: environment.api_base.as_deref(),
        environment_api_key: None,
    };
    assert_eq!(
        accessors.api_base(None).as_deref(),
        Some("https://global.test")
    );
    assert_eq!(
        accessors.api_base(Some("https://explicit.test")).as_deref(),
        Some("https://explicit.test")
    );
    let environment_only = ModelInfoAccessors {
        global_api_base: None,
        global_api_key: None,
        environment_api_base: environment.api_base.as_deref(),
        environment_api_key: None,
    };
    assert_eq!(environment_only.api_base(None).as_deref(), Some(BASE));
}
