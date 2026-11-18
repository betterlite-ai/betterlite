async fn load_stored(
    repository: &dyn SettingsRepository,
) -> Result<StoredSettings, RoiSettingsError> {
    let Some(parameter) = repository
        .param(SETTINGS_KEY)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))?
    else {
        return Ok(StoredSettings::default());
    };
    let bytes = serde_json::to_vec(&parameter)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    let stored: StoredSettings =
        serde_json::from_slice(&bytes).map_err(|_error| RoiSettingsError::StoredSettings)?;
    let backfill_valid = (1..=3650).contains(&stored.backfill_days);
    let interval = stored.update_interval_minutes;
    let interval_valid = interval.is_finite() && (0.0_f64..=43_200.0_f64).contains(&interval);
    if !backfill_valid || !interval_valid {
        return Err(RoiSettingsError::StoredSettings);
    }
    Ok(stored)
}

async fn load_settings(
    repository: &dyn SettingsRepository,
    cipher: &DbCipher,
) -> Result<ROISettings, RoiSettingsError> {
    let stored = load_stored(repository).await?;
    let settings = ROISettings {
        github_api_url: stored.github_api_url,
        github_token: SecretString::from(decrypt(cipher, &stored.github_token)),
        estimator_key: SecretString::from(decrypt(cipher, &stored.estimator_key)),
        repos: stored.repos,
        estimator_model: stored.estimator_model,
        estimator_prompt: stored.estimator_prompt,
        backfill_days: stored.backfill_days,
        update_interval_minutes: stored.update_interval_minutes,
        identity_map: stored.identity_map,
    };
    settings
        .validate()
        .map_err(|_error| RoiSettingsError::StoredSettings)
}

async fn save_settings(
    repository: &dyn SettingsRepository,
    stored: &StoredSettings,
) -> Result<(), RoiSettingsError> {
    let bytes = serde_json::to_vec(stored)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    let value: JsonData = serde_json::from_slice(&bytes)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    repository
        .set_param(SETTINGS_KEY, value)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))
}

pub async fn load_report(
    repository: &dyn SettingsRepository,
) -> Result<Option<ROIReport>, RoiSettingsError> {
    let Some(parameter) = repository
        .param(REPORT_KEY)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))?
    else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&parameter)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    let report = serde_json::from_slice(&bytes).map_err(|_error| RoiSettingsError::StoredReport)?;
    Ok(Some(report))
}

fn public_settings(settings: &ROISettings, catalog: &dyn RouterCatalog) -> ROISettingsResponse {
    let available_models = router_models(catalog);
    let ready = !settings.repos.is_empty()
        && !settings.estimator_model.is_empty()
        && available_models.contains(&settings.estimator_model);
    ROISettingsResponse {
        github_api_url: settings.github_api_url.clone(),
        repos: settings.repos.clone(),
        estimator_model: settings.estimator_model.clone(),
        estimator_prompt: settings.estimator_prompt.clone(),
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        has_estimator_key: !settings.estimator_key.expose_secret().is_empty(),
        identity_map: settings.identity_map.clone(),
        has_github_token: !settings.github_token.expose_secret().is_empty(),
        default_prompt: DEFAULT_PROMPT.to_owned(),
        available_models,
        ready,
    }
}

fn check_json(body: &[u8]) -> Result<(), RoiSettingsError> {
    if body.is_empty() {
        return Err(RoiSettingsError::EmptyBody);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    IgnoredAny::deserialize(&mut deserializer).map_err(|_error| RoiSettingsError::InvalidJson)?;
    deserializer
        .end()
        .map_err(|_error| RoiSettingsError::InvalidJson)
}

fn decode_body<T>(body: &[u8]) -> Result<T, RoiSettingsError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| RoiSettingsError::Body(Box::new(source)))
}

fn patched_secrets(
    patch: &ROISettingsUpdate,
    presence: &PatchPresence,
    stored: &StoredSettings,
    current: &ROISettings,
    cipher: &DbCipher,
) -> Result<PatchedSecrets, RoiSettingsError> {
    let url = patch
        .github_api_url
        .as_deref()
        .unwrap_or(&current.github_api_url);
    let url_changed = url.trim_end_matches('/') != current.github_api_url.trim_end_matches('/');
    let (token, encrypted_token) = if presence.github_token.is_present() {
        let token = secret_value(patch.github_token.as_ref());
        let encrypted = encrypt(cipher, &token)?;
        (token, encrypted)
    } else if url_changed {
        (String::new(), String::new())
    } else {
        (
            current.github_token.expose_secret().to_owned(),
            stored.github_token.clone(),
        )
    };
    let estimator_key = if presence.estimator_key.is_present() {
        secret_value(patch.estimator_key.as_ref())
    } else {
        current.estimator_key.expose_secret().to_owned()
    };
    let encrypted_estimator_key = encrypt(cipher, &estimator_key)?;
    Ok(PatchedSecrets {
        token,
        encrypted_token,
        estimator_key,
        encrypted_estimator_key,
    })
}

fn secret_value(value: Option<&SecretString>) -> String {
    value.map_or_else(String::new, |secret| secret.expose_secret().to_owned())
}

fn build_settings(
    patch: ROISettingsUpdate,
    current: &ROISettings,
    secrets: &PatchedSecrets,
) -> Result<ROISettings, RoiSettingsError> {
    let settings = ROISettings {
        github_api_url: patch
            .github_api_url
            .clone()
            .unwrap_or_else(|| current.github_api_url.clone()),
        github_token: SecretString::from(secrets.token.clone()),
        estimator_key: SecretString::from(secrets.estimator_key.clone()),
        update_interval_minutes: patch
            .update_interval_minutes
            .unwrap_or(current.update_interval_minutes),
        repos: patch.repos.unwrap_or_else(|| current.repos.clone()),
        estimator_model: patch
            .estimator_model
            .unwrap_or_else(|| current.estimator_model.clone()),
        estimator_prompt: patch
            .estimator_prompt
            .unwrap_or_else(|| current.estimator_prompt.clone()),
        backfill_days: patch.backfill_days.unwrap_or(current.backfill_days),
        identity_map: current.identity_map.clone(),
    };
    settings.validate().map_err(RoiSettingsError::Validation)
}
