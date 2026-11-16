use std::sync::Mutex;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_openai::vector_stores::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_sources::getters::VertexParams;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};
use crate::vertex_veo_request::environment::validate_environment;
use crate::vertex_veo_request::error::VeoRequestError;

// records what validate_environment hands the mint: LiteLLM's get_access_token(credentials=..., project_id=...)
struct RecordingMint {
    calls: Mutex<Vec<(Option<String>, Option<String>)>>,
    fails: bool,
}

impl RecordingMint {
    fn new(fails: bool) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fails,
        }
    }

    fn calls(&self) -> Vec<(Option<String>, Option<String>)> {
        self.calls.lock().expect("the mint records").clone()
    }
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.calls.lock().expect("the mint records").push((
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
            project.map(str::to_owned),
        ));
        if self.fails {
            return Box::pin(std::future::ready(Err(CredentialCacheError::MissingToken)));
        }
        Box::pin(std::future::ready(Ok(AccessToken {
            token: SecretString::from("minted-token"),
            project: project.unwrap_or("project-1").to_owned(),
        })))
    }
}

fn credentialed_params<'params>(
    project: Option<&'params str>,
    credentials: &'params ConfiguredCredentials,
) -> VertexParams<'params> {
    VertexParams {
        project,
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: Some(credentials),
        ai_credentials: None,
    }
}

#[tokio::test]
async fn headers_start_with_the_minted_bearer_and_json_defaults() {
    let settings = VertexSettings::default();
    let credentials = ConfiguredCredentials::Text(SecretString::from("a-secret"));
    let mint = RecordingMint::new(false);
    let headers = validate_environment(
        Headers::new(),
        credentialed_params(Some("project-1"), &credentials),
        &settings,
        &mint,
    )
    .await
    .expect("the headers are built");
    assert_eq!(
        headers
            .get("Authorization")
            .map(|value| value.expose_secret()),
        Some("Bearer minted-token")
    );
    assert_eq!(
        headers
            .get("Content-Type")
            .map(|value| value.expose_secret()),
        Some("application/json")
    );
    assert_eq!(
        mint.calls(),
        vec![(Some("a-secret".to_owned()), Some("project-1".to_owned()))]
    );
}

#[tokio::test]
async fn caller_headers_override_the_defaults_and_are_kept() {
    let settings = VertexSettings::default();
    let credentials = ConfiguredCredentials::Text(SecretString::from("a-secret"));
    let mint = RecordingMint::new(false);
    let caller = Headers::from([
        (
            "Authorization".to_owned(),
            SecretString::from("Bearer caller"),
        ),
        ("Content-Type".to_owned(), SecretString::from("text/plain")),
        ("X-Extra".to_owned(), SecretString::from("1")),
    ]);
    let headers = validate_environment(
        caller,
        credentialed_params(Some("project-1"), &credentials),
        &settings,
        &mint,
    )
    .await
    .expect("the headers are built");
    assert_eq!(
        headers
            .get("Authorization")
            .map(|value| value.expose_secret()),
        Some("Bearer caller")
    );
    assert_eq!(
        headers
            .get("Content-Type")
            .map(|value| value.expose_secret()),
        Some("text/plain")
    );
    assert_eq!(
        headers.get("X-Extra").map(|value| value.expose_secret()),
        Some("1")
    );
}

#[tokio::test]
async fn a_failed_mint_is_reported_as_a_mint_error() {
    let settings = VertexSettings::default();
    let credentials = ConfiguredCredentials::Text(SecretString::from("a-secret"));
    let mint = RecordingMint::new(true);
    let error = validate_environment(
        Headers::new(),
        credentialed_params(Some("project-1"), &credentials),
        &settings,
        &mint,
    )
    .await
    .expect_err("the mint fails");
    assert!(matches!(error, VeoRequestError::Mint(_)));
}
