#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::ImageIdentityError;
use super::settings::{
    ImageEnvironment, ImageIdentitySettings, ImageInstance, ImageModule, ImageSecrets,
};
use super::size_aspect_ratio;
use super::target::{ImageTarget, ValidationRequest, complete_url, validate_environment};
use crate::credential_sources::getters::VertexParams;
use crate::credential_sources::sdk::CredentialClock;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;

const STANDARD_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-flash-image:generateContent";

fn credential(text: &str) -> ConfiguredCredentials {
    ConfiguredCredentials::from_text(text.to_owned()).expect("plain text is a stored credential")
}

fn full_settings() -> ImageIdentitySettings {
    ImageIdentitySettings {
        instance: ImageInstance {
            project: Some("instance-project".to_owned()),
            location: Some("instance-location".to_owned()),
            credentials: Some(credential("instance-credentials")),
        },
        environment: ImageEnvironment {
            project: Some("env-project".to_owned()),
            location: Some("env-location".to_owned()),
            legacy_location: Some("legacy-env-location".to_owned()),
            credentials: Some(credential("env-credentials")),
            application_credentials: Some(credential("/keys/application.json")),
        },
        module: ImageModule {
            project: Some("module-project".to_owned()),
            location: Some("module-location".to_owned()),
            credentials: Some(credential("module-credentials")),
        },
        secrets: ImageSecrets {
            project: Some("secret-project".to_owned()),
            location: Some("secret-location".to_owned()),
            legacy_location: Some("legacy-secret-location".to_owned()),
            credentials: Some(credential("secret-credentials")),
        },
    }
}

fn target_settings() -> ImageIdentitySettings {
    ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
            application_credentials: None,
        },
        module: ImageModule {
            project: Some("proj".to_owned()),
            location: Some("us-central1".to_owned()),
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    }
}

fn text(credentials: Option<&ConfiguredCredentials>) -> Option<&str> {
    credentials.and_then(ConfiguredCredentials::as_text)
}

#[rstest]
#[case("1024x1024", "1:1")]
#[case("1792x1024", "16:9")]
#[case("1024x1792", "9:16")]
#[case("1280x896", "4:3")]
#[case("896x1280", "3:4")]
#[case("999x999", "1:1")]
#[case("", "1:1")]
#[case("1024X1024", "1:1")]
fn the_size_map_holds_five_sizes_and_falls_back_to_square(#[case] size: &str, #[case] ratio: &str) {
    assert_eq!(size_aspect_ratio(size), ratio);
}

#[test]
fn the_project_tiers_follow_the_safe_getter_then_fallback_chain() {
    let mut settings = full_settings();
    let both = VertexParams {
        project: Some("param-project"),
        ai_project: Some("ai-param-project"),
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    assert_eq!(settings.resolve_project(both), Some("param-project"));
    let second = VertexParams {
        project: None,
        ai_project: Some("ai-param-project"),
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    assert_eq!(settings.resolve_project(second), Some("ai-param-project"));
    assert_eq!(
        settings.resolve_project(VertexParams::default()),
        Some("module-project")
    );
    settings.module.project = None;
    assert_eq!(
        settings.resolve_project(VertexParams::default()),
        Some("secret-project")
    );
    settings.secrets.project = None;
    assert_eq!(
        settings.resolve_project(VertexParams::default()),
        Some("instance-project")
    );
    settings.instance.project = None;
    assert_eq!(
        settings.resolve_project(VertexParams::default()),
        Some("env-project")
    );
    settings.environment.project = None;
    assert_eq!(settings.resolve_project(VertexParams::default()), None);
}

#[test]
fn an_empty_project_parameter_falls_through_to_the_ai_spelling() {
    let settings = full_settings();
    let params = VertexParams {
        project: Some(""),
        ai_project: Some("ai-param-project"),
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    assert_eq!(settings.resolve_project(params), Some("ai-param-project"));
}

#[test]
fn an_empty_location_parameter_falls_through_to_the_ai_spelling() {
    let settings = full_settings();
    let params = VertexParams {
        project: None,
        ai_project: None,
        location: Some(""),
        ai_location: Some("ai-param-location"),
        credentials: None,
        ai_credentials: None,
    };
    assert_eq!(settings.resolve_location(params), Some("ai-param-location"));
}

#[test]
fn the_location_tiers_read_both_secret_spellings_before_the_environment() {
    let mut settings = full_settings();
    assert_eq!(
        settings.resolve_location(VertexParams::default()),
        Some("module-location")
    );
    settings.module.location = None;
    assert_eq!(
        settings.resolve_location(VertexParams::default()),
        Some("secret-location")
    );
    settings.secrets.location = None;
    assert_eq!(
        settings.resolve_location(VertexParams::default()),
        Some("legacy-secret-location")
    );
    settings.secrets.legacy_location = None;
    assert_eq!(
        settings.resolve_location(VertexParams::default()),
        Some("instance-location")
    );
    settings.instance.location = None;
    assert_eq!(
        settings.resolve_location(VertexParams::default()),
        Some("env-location")
    );
    settings.environment.location = None;
    assert_eq!(
        settings.resolve_location(VertexParams::default()),
        Some("legacy-env-location")
    );
    settings.environment.legacy_location = None;
    assert_eq!(settings.resolve_location(VertexParams::default()), None);
}

#[test]
fn the_module_credentials_sit_between_the_environment_and_the_application_defaults() {
    let mut settings = full_settings();
    assert_eq!(
        text(settings.resolve_credentials(VertexParams::default())),
        Some("secret-credentials")
    );
    settings.secrets.credentials = None;
    assert_eq!(
        text(settings.resolve_credentials(VertexParams::default())),
        Some("instance-credentials")
    );
    settings.instance.credentials = None;
    assert_eq!(
        text(settings.resolve_credentials(VertexParams::default())),
        Some("env-credentials")
    );
    settings.environment.credentials = None;
    assert_eq!(
        text(settings.resolve_credentials(VertexParams::default())),
        Some("module-credentials")
    );
    settings.module.credentials = None;
    assert_eq!(
        text(settings.resolve_credentials(VertexParams::default())),
        Some("/keys/application.json")
    );
    settings.environment.application_credentials = None;
    assert_eq!(settings.resolve_credentials(VertexParams::default()), None);
}

#[test]
fn a_parameter_credential_beats_every_fallback() {
    let settings = full_settings();
    let stored = credential("param-credentials");
    let params = VertexParams {
        project: None,
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: Some(&stored),
        ai_credentials: None,
    };
    assert_eq!(
        text(settings.resolve_credentials(params)),
        Some("param-credentials")
    );
}

#[test]
fn a_custom_base_is_the_complete_url_with_trailing_slashes_trimmed() {
    let settings = ImageIdentitySettings::default();
    let target = ImageTarget {
        model: "gemini-2.5-flash-image",
        api_base: Some("https://proxy.example/v1beta///"),
        params: VertexParams::default(),
    };
    assert_eq!(
        complete_url(&settings, &target).expect("a custom base needs no project"),
        "https://proxy.example/v1beta"
    );
}

#[test]
fn an_empty_custom_base_cannot_become_the_url() {
    let target = ImageTarget {
        model: "gemini-2.5-flash-image",
        api_base: Some(""),
        params: VertexParams::default(),
    };
    assert_eq!(
        complete_url(&target_settings(), &target).expect("the standard URL still builds"),
        STANDARD_URL
    );
}

#[test]
fn the_vertex_ai_prefix_is_stripped_from_the_url_model() {
    let target = ImageTarget {
        model: "vertex_ai/gemini-2.5-flash-image",
        api_base: None,
        params: VertexParams::default(),
    };
    assert_eq!(
        complete_url(&target_settings(), &target).expect("the standard URL builds"),
        STANDARD_URL
    );
}

#[test]
fn the_global_location_uses_the_control_plane_host() {
    let settings = ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
            application_credentials: None,
        },
        module: ImageModule {
            project: Some("proj".to_owned()),
            location: Some("global".to_owned()),
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    };
    let target = ImageTarget {
        model: "gemini-2.5-flash-image",
        api_base: None,
        params: VertexParams::default(),
    };
    assert_eq!(
        complete_url(&settings, &target).expect("global is a valid location"),
        "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/google/models/gemini-2.5-flash-image:generateContent"
    );
}

#[test]
fn a_required_project_or_location_that_is_absent_or_empty_is_one_error() {
    let target = ImageTarget {
        model: "gemini-2.5-flash-image",
        api_base: None,
        params: VertexParams::default(),
    };
    let missing = ImageIdentitySettings::default();
    assert!(matches!(
        complete_url(&missing, &target),
        Err(ImageIdentityError::MissingTarget)
    ));
    let project_only = ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
            application_credentials: None,
        },
        module: ImageModule {
            project: Some("proj".to_owned()),
            location: None,
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    };
    assert!(matches!(
        complete_url(&project_only, &target),
        Err(ImageIdentityError::MissingTarget)
    ));
    let empty_project = ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
            application_credentials: None,
        },
        module: ImageModule {
            project: Some(String::new()),
            location: Some("us-central1".to_owned()),
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    };
    assert!(matches!(
        complete_url(&empty_project, &target),
        Err(ImageIdentityError::MissingTarget)
    ));
}

#[test]
fn a_location_outside_the_lowercase_host_alphabet_never_chooses_the_host() {
    let settings = ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
            application_credentials: None,
        },
        module: ImageModule {
            project: Some("proj".to_owned()),
            location: Some("us-central1.evil.example".to_owned()),
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    };
    let target = ImageTarget {
        model: "gemini-2.5-flash-image",
        api_base: None,
        params: VertexParams::default(),
    };
    assert!(matches!(
        complete_url(&settings, &target),
        Err(ImageIdentityError::Location(
            VertexUrlError::InvalidLocation
        ))
    ));
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

type Asked = Arc<Mutex<Vec<(Option<String>, Option<String>)>>>;

struct RecordingExecutor {
    asked: Asked,
    fails: bool,
}

impl CredentialExecutor for RecordingExecutor {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.asked
            .lock()
            .expect("the record is not poisoned")
            .push((
                text(credentials).map(str::to_owned),
                project.map(str::to_owned),
            ));
        if self.fails {
            return Box::pin(ready(Err(CredentialCacheError::MissingCredentials)));
        }
        let snapshot = CredentialSnapshot {
            token: Some(SecretString::from("vertex-token")),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some("proj".to_owned()),
        };
        Box::pin(ready(Ok(LoadedCredential {
            credential: Some(Arc::new(snapshot)),
            project: Some("proj".to_owned()),
        })))
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(ready(Err(CredentialCacheError::MissingToken)))
    }
}

fn cache(asked: &Asked, fails: bool) -> CredentialCache {
    CredentialCache::new(
        Arc::new(FixedClock),
        Arc::new(RecordingExecutor {
            asked: Arc::clone(asked),
            fails,
        }),
    )
}

fn caller_headers() -> BTreeMap<String, String> {
    BTreeMap::from([("x-goog-user-project".to_owned(), "billing".to_owned())])
}

#[tokio::test]
async fn a_custom_base_skips_credentials_and_returns_the_callers_headers() {
    let asked = Asked::default();
    let cache = cache(&asked, false);
    let headers = caller_headers();
    let request = ValidationRequest {
        params: VertexParams::default(),
        api_base: Some("https://proxy.example"),
        params_api_base: None,
        headers: &headers,
    };
    let resolved = validate_environment(&cache, &ImageIdentitySettings::default(), &request)
        .await
        .expect("a custom base needs no credentials");
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved
            .get("x-goog-user-project")
            .map(|value| value.expose_secret()),
        Some("billing")
    );
    assert!(asked.lock().expect("the record is not poisoned").is_empty());
}

#[tokio::test]
async fn an_empty_parameter_base_alone_is_the_same_as_no_base() {
    let asked = Asked::default();
    let cache = cache(&asked, false);
    let headers = caller_headers();
    let request = ValidationRequest {
        params: VertexParams::default(),
        api_base: None,
        params_api_base: Some(""),
        headers: &headers,
    };
    let resolved = validate_environment(&cache, &ImageIdentitySettings::default(), &request)
        .await
        .expect("the executor answers a token");
    assert_eq!(
        resolved
            .get("Authorization")
            .map(|value| value.expose_secret()),
        Some("Bearer vertex-token")
    );
    assert_eq!(asked.lock().expect("the record is not poisoned").len(), 1);
}

#[tokio::test]
async fn an_empty_api_base_argument_also_skips_credentials() {
    let asked = Asked::default();
    let cache = cache(&asked, false);
    let headers = BTreeMap::new();
    let request = ValidationRequest {
        params: VertexParams::default(),
        api_base: Some(""),
        params_api_base: None,
        headers: &headers,
    };
    let resolved = validate_environment(&cache, &ImageIdentitySettings::default(), &request)
        .await
        .expect("an empty base is still not None");
    assert!(resolved.is_empty());
}

#[tokio::test]
async fn the_normal_path_mints_a_token_for_the_resolved_project_and_credentials() {
    let asked = Asked::default();
    let cache = cache(&asked, false);
    let headers = caller_headers();
    let request = ValidationRequest {
        params: VertexParams::default(),
        api_base: None,
        params_api_base: None,
        headers: &headers,
    };
    let settings = ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: Some("env-project".to_owned()),
            location: None,
            legacy_location: None,
            credentials: Some(credential("env-credentials")),
            application_credentials: None,
        },
        module: ImageModule {
            project: None,
            location: None,
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    };
    let resolved = validate_environment(&cache, &settings, &request)
        .await
        .expect("the executor answers a token");
    assert_eq!(
        resolved
            .get("Content-Type")
            .map(|value| value.expose_secret()),
        Some("application/json")
    );
    assert_eq!(
        resolved
            .get("Authorization")
            .map(|value| value.expose_secret()),
        Some("Bearer vertex-token")
    );
    assert_eq!(
        resolved
            .get("x-goog-user-project")
            .map(|value| value.expose_secret()),
        Some("billing")
    );
    let recorded = asked.lock().expect("the record is not poisoned");
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0],
        (
            Some("env-credentials".to_owned()),
            Some("env-project".to_owned())
        )
    );
}

#[tokio::test]
async fn a_credential_failure_reaches_the_caller_as_the_cache_error() {
    let asked = Asked::default();
    let cache = cache(&asked, true);
    let headers = BTreeMap::new();
    let request = ValidationRequest {
        params: VertexParams::default(),
        api_base: None,
        params_api_base: None,
        headers: &headers,
    };
    let error = validate_environment(&cache, &ImageIdentitySettings::default(), &request)
        .await
        .expect_err("the executor has no credentials");
    assert!(matches!(
        error,
        ImageIdentityError::Auth(CredentialCacheError::MissingCredentials)
    ));
}
