use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::job::{Lens, LensList};
use betterlite_lens::models::messages::RunRequest;
use betterlite_lens::models::settings::{LensSettings, Scope};
use betterlite_lens::sources::activity_availability::ActivityAvailability;
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::{can_access, queue_job};
use rust_decimal::Decimal;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::lookup::{get_lens, lens_repository, required};
use crate::lens_access::model::validate_model;
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_access::storage::source_reader;
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::{
    LensMutationError, LensRouteError, route_key_scope, update_failure,
};

impl<Db, Store> LensRoutesState<Db, Store>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    pub(crate) async fn list_lenses(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<LensList>, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        let repository = lens_repository(&state)?;
        let mut lenses = repository.lenses().await?;
        lenses.retain(|lens| can_access(&scope, &lens.scope));
        let mut workers = repository.workers().await?;
        workers.retain(|worker| can_access(&scope, &worker.scope));
        Ok(Json(LensList {
            lenses,
            workers,
            tracing_enabled: state.storage.is_some(),
        }))
    }

    pub(crate) async fn activity_available(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<ActivityAvailability>, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        if state.storage.is_none() {
            return Ok(Json(ActivityAvailability::default()));
        }
        let reader = source_reader(state.storage.as_ref())?;
        Ok(Json(reader.availability(&scope).await?))
    }

    pub(crate) async fn list_agents(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<Vec<String>>, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        let Some(storage) = state.storage.as_ref() else {
            return Ok(Json(Vec::new()));
        };
        Ok(Json(source_reader(Some(storage))?.agents(&scope).await?))
    }

    pub(crate) async fn update_lens(
        State(state): State<Self>,
        Path(lens_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>> {
        let (settings, key, scope) = decoded_scope::<Store>(caller, &body)?;
        let repository = lens_repository(&state)?;
        get_lens(repository, &lens_id, &scope).await?;
        validate_model(&settings, &key, &state)?;
        let updated = repository
            .update(
                &lens_id,
                |lens| with_settings(lens, settings.clone()),
                DEFAULT_ATTEMPTS,
                false,
            )
            .await
            .map_err(update_failure)?;
        Ok(Json(required(updated)?))
    }
}

pub(crate) async fn create_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let (settings, key, scope) = decoded_scope::<Store>(caller, &body)?;
    validate_model(&settings, &key, &state)?;
    let now = state.clock.now();
    let lens = Lens {
        id: state.ids.new_id(),
        scope,
        settings,
        revision: 1,
        version: 0,
        created_at: now,
        next_run_at: now,
        last_scan_at: None,
        jobs: Vec::new(),
        findings: Vec::new(),
        budget_month: now.format("%Y-%m").to_string(),
        spent: Decimal::ZERO,
    };
    let queued = queue_job(&lens, now, state.ids.new_id(), &RunRequest::default())
        .map_err(LensMutationError::from)?;
    let repository = lens_repository(&state)?;
    Ok(Json(repository.create(queued).await?))
}

pub(crate) async fn read_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(lens_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let (_, scope) = route_key_scope::<Store>(caller, false)?;
    let repository = lens_repository(&state)?;
    Ok(Json(get_lens(repository, &lens_id, &scope).await?))
}

fn decoded_scope<Store>(
    caller: Option<Extension<AuthContext>>,
    body: &[u8],
) -> Result<(LensSettings, UserAPIKeyAuth, Scope), LensRouteError<<Store as Storage>::Error>>
where
    Store: Storage,
{
    let settings: LensSettings = decode(body)?;
    let (key, scope) = route_key_scope::<Store>(caller, true)?;
    Ok((settings, key, scope))
}

fn with_settings(lens: Lens, settings: LensSettings) -> Result<Lens, LensMutationError> {
    let revision = lens
        .revision
        .checked_add(1)
        .ok_or(LensMutationError::Revision)?;
    Ok(Lens {
        settings,
        revision,
        ..lens
    })
}

mod tests;
