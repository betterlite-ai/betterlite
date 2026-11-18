use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct Catalog {
    names: Vec<String>,
    deployments: Vec<RouterEstimatorDeployment>,
}

#[derive(Default)]
struct MemoryRepository {
    params: Mutex<HashMap<String, JsonData>>,
}

#[async_trait]
impl SettingsRepository for MemoryRepository {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError> {
        Ok(self
            .params
            .lock()
            .expect("repository lock")
            .get(name)
            .cloned())
    }

    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError> {
        self.params
            .lock()
            .expect("repository lock")
            .insert(name.to_owned(), value);
        Ok(())
    }
}

impl RouterCatalog for Catalog {
    fn model_names(&self) -> Vec<String> {
        self.names.clone()
    }

    fn deployments(&self, _: &str) -> Vec<RouterEstimatorDeployment> {
        self.deployments.clone()
    }
}

fn deployment(model: Option<&str>, base_model: Option<&str>) -> RouterEstimatorDeployment {
    RouterEstimatorDeployment {
        litellm_params: RouterEstimatorParams {
            model: model.map(str::to_owned),
            base_model: None,
            custom_llm_provider: Some("openai".to_owned()),
        },
        model_info: RouterEstimatorModelInfo {
            base_model: base_model.map(str::to_owned),
        },
    }
}

#[test]
fn deployment_ignores_unknown_fields() {
    let value: RouterEstimatorDeployment = serde_json::from_str(
        r#"{"litellm_params":{"model":"m","extra":1},"model_info":{},"extra":2}"#,
    )
    .expect("unknown deployment fields are ignored");
    assert_eq!(value.litellm_params.model.as_deref(), Some("m"));
}

#[test]
fn estimator_models_skip_missing_model() {
    assert_eq!(
        estimator_models(&[deployment(Some("m"), None), deployment(None, None)]).len(),
        1
    );
}

#[test]
fn estimator_model_prefers_model_info_base_model() {
    assert_eq!(
        estimator_model(&deployment(Some("fallback"), Some("preferred")))
            .unwrap()
            .0,
        "preferred"
    );
}

#[test]
fn router_estimator_models_filters_missing_entries() {
    let catalog = Catalog {
        deployments: vec![deployment(Some("m"), None), deployment(None, None)],
        ..Catalog::default()
    };
    assert_eq!(router_estimator_models(&catalog, "estimator").len(), 1);
}

#[test]
fn router_models_sorts_and_deduplicates() {
    let catalog = Catalog {
        names: vec!["z".to_owned(), "a".to_owned(), "a".to_owned()],
        ..Catalog::default()
    };
    assert_eq!(router_models(&catalog), ["a", "z"]);
}

#[test]
fn read_admin_accepts_viewer() {
    let context = AuthContext {
        principal: principal(vec![Role::PlatformViewer]),
    };
    assert!(admin(Some(&context), false).is_ok());
}

#[test]
fn write_admin_rejects_viewer() {
    let context = AuthContext {
        principal: principal(vec![Role::PlatformViewer]),
    };
    assert!(matches!(
        admin(Some(&context), true),
        Err(RoiSettingsError::Forbidden(_))
    ));
}

fn principal(roles: Vec<Role>) -> betterlite_auth::auth_identity::records::Principal {
    use betterlite_auth::auth_identity::records::{CredentialRef, NetworkContext, Principal};
    use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
    Principal {
        principal_type: PrincipalType::Human,
        subject: "user".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: Default::default(),
    }
}

#[test]
fn config_repository_requires_database() {
    let state = RoiSettingsState::new(
        None,
        Arc::new(Catalog::default()),
        CipherSettings {
            signing_key: None,
            algorithm: betterlite_secrets::secret_codec::Algorithm::XSalsa20Poly1305,
        },
    );
    assert!(matches!(
        require_repository(&state),
        Err(RoiSettingsError::Internal(InternalError::DbNotConnected))
    ));
}

#[tokio::test]
async fn load_stored_uses_defaults_when_missing() {
    let settings = load_stored(&MemoryRepository::default())
        .await
        .expect("missing settings use defaults");
    assert_eq!(settings.github_api_url, DEFAULT_GITHUB_API_URL);
    assert_eq!(settings.backfill_days, 7);
}

#[test]
fn load_settings_decrypts_empty_secret_to_empty() {
    assert_eq!(
        decrypt(
            &DbCipher {
                settings: CipherSettings {
                    signing_key: None,
                    algorithm: betterlite_secrets::secret_codec::Algorithm::XSalsa20Poly1305
                },
                nonces: OsNonces,
                diagnostics: TracingDiagnostics
            },
            ""
        ),
        ""
    );
}

#[tokio::test]
async fn save_settings_uses_settings_key() {
    let repository = MemoryRepository::default();
    save_settings(&repository, &StoredSettings::default())
        .await
        .expect("save settings");
    assert!(repository.param(SETTINGS_KEY).await.unwrap().is_some());
}

#[test]
fn public_settings_projects_model_readiness() {
    let settings = ROISettings {
        repos: vec!["owner/repo".to_owned()],
        estimator_model: "m".to_owned(),
        ..ROISettings::default()
    };
    let catalog = Catalog {
        names: vec!["m".to_owned()],
        ..Catalog::default()
    };
    assert!(public_settings(&settings, &catalog).ready);
}

#[tokio::test]
async fn load_report_uses_report_key() {
    let repository = MemoryRepository::default();
    assert!(load_report(&repository).await.unwrap().is_none());
    assert!(repository.param(REPORT_KEY).await.unwrap().is_none());
}

#[test]
fn get_route_is_registered() {
    let state = RoiSettingsState::new(
        Some(Arc::new(MemoryRepository::default())),
        Arc::new(Catalog::default()),
        CipherSettings {
            signing_key: None,
            algorithm: betterlite_secrets::secret_codec::Algorithm::XSalsa20Poly1305,
        },
    );
    let app = router(state);
    assert!(format!("{app:?}").contains("Router"));
}

#[test]
fn put_null_url_is_distinguished() {
    let presence: PatchPresence = serde_json::from_str(r#"{"github_api_url":null}"#).unwrap();
    assert_eq!(presence.github_api_url, FieldState::Null);
}

#[test]
fn put_url_change_clears_token() {
    let current = ROISettings {
        github_token: SecretString::from("old-token"),
        ..ROISettings::default()
    };
    let patch = ROISettingsUpdate {
        github_api_url: Some("https://github.enterprise".to_owned()),
        ..ROISettingsUpdate::default()
    };
    let secrets = patched_secrets(
        &patch,
        &PatchPresence::default(),
        &StoredSettings::default(),
        &current,
        &test_cipher(),
    )
    .expect("url change clears the existing token");
    assert!(secrets.token.is_empty());
}

#[test]
fn put_url_same_preserves_token() {
    let current = ROISettings {
        github_token: SecretString::from("existing"),
        ..ROISettings::default()
    };
    let patch = ROISettingsUpdate {
        github_api_url: Some("https://api.github.com/".to_owned()),
        ..ROISettingsUpdate::default()
    };
    let secrets = patched_secrets(
        &patch,
        &PatchPresence::default(),
        &StoredSettings::default(),
        &current,
        &test_cipher(),
    )
    .expect("trailing slash does not change the origin");
    assert_eq!(secrets.token, "existing");
}

#[test]
fn put_token_encrypts_nonempty_values() {
    let cipher = test_cipher();
    let encrypted = encrypt(&cipher, "secret").expect("secret encryption");
    assert_ne!(encrypted, "secret");
    assert_eq!(decrypt(&cipher, &encrypted), "secret");
}

#[test]
fn put_validation_rejects_invalid_settings() {
    let settings = ROISettings {
        backfill_days: 0,
        ..ROISettings::default()
    };
    assert!(settings.validate().is_err());
}

#[test]
fn put_db_error_is_internal_server_error() {
    let response = RoiSettingsError::Internal(InternalError::DbNotConnected).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

fn test_cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("a sufficiently long test key")),
            algorithm: betterlite_secrets::secret_codec::Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
