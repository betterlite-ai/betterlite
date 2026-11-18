fn require_repository(
    state: &RoiSettingsState,
) -> Result<&dyn SettingsRepository, RoiSettingsError> {
    state
        .repository
        .as_deref()
        .ok_or(RoiSettingsError::Internal(InternalError::DbNotConnected))
}

async fn get_settings(
    State(state): State<RoiSettingsState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ROISettingsResponse>, RoiSettingsError> {
    admin(caller.as_ref().map(|extension| &extension.0), false)?;
    let repository = require_repository(&state)?;
    let settings = load_settings(repository, &state.cipher).await?;
    Ok(Json(public_settings(&settings, state.catalog.as_ref())))
}

async fn update_settings(
    State(state): State<RoiSettingsState>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<ROISettingsResponse>, RoiSettingsError> {
    check_json(&body)?;
    admin(caller.as_ref().map(|extension| &extension.0), true)?;
    let repository = require_repository(&state)?;
    let (patch, presence): (ROISettingsUpdate, PatchPresence) = decode_body(&body)?;
    let issues = request_issues(&patch);
    if !issues.is_empty() {
        return Err(RoiSettingsError::Request(issues));
    }
    let stored = load_stored(repository).await?;
    let current = load_settings(repository, &state.cipher).await?;
    if presence.github_api_url == FieldState::Null {
        return Err(RoiSettingsError::NullUrl);
    }
    let secrets = patched_secrets(&patch, &presence, &stored, &current, &state.cipher)?;
    let settings = build_settings(patch, &current, &secrets)?;
    let stored_settings = StoredSettings {
        github_api_url: settings.github_api_url.clone(),
        github_token: secrets.encrypted_token,
        estimator_key: secrets.encrypted_estimator_key,
        repos: settings.repos.clone(),
        estimator_model: settings.estimator_model.clone(),
        estimator_prompt: settings.estimator_prompt.clone(),
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        identity_map: settings.identity_map.clone(),
    };
    save_settings(repository, &stored_settings).await?;
    Ok(Json(public_settings(&settings, state.catalog.as_ref())))
}
