use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::update::DEFAULT_ATTEMPTS;
use betterlite_lens::models::job::{Lens, LensList};
use betterlite_lens::models::messages::RunRequest;
use betterlite_lens::models::settings::LensSettings;
use betterlite_lens::sources::activity_availability::ActivityAvailability;
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::state::transitions::{can_access, queue_job};
use rust_decimal::Decimal;

use crate::auth::auth_identity::AuthContext;
use crate::lens_access::error::LensAccessError;
use crate::lens_access::lookup::{get_lens, lens_repository, required};
use crate::lens_access::model::validate_model;
use crate::lens_access::scope::{caller_key, user_scope};
use crate::lens_access::state::LensRoutesState;
use crate::lens_access::storage::source_reader;
use crate::lens_admin_routes::body::decode;
use crate::lens_admin_routes::error::{LensMutationError, LensRouteError, update_failure};

pub(crate) async fn list_lenses<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<LensList>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let scope = user_scope(&caller_key(&context.principal)?, false)?;
    let repository = lens_repository(&state)?;
    let lenses = repository.lenses().await?;
    let workers = repository.workers().await?;
    let allowed_lenses = lenses
        .into_iter()
        .filter(|lens| can_access(&scope, &lens.scope))
        .collect();
    let allowed_workers = workers
        .into_iter()
        .filter(|worker| can_access(&scope, &worker.scope))
        .collect();
    Ok(Json(LensList {
        lenses: allowed_lenses,
        workers: allowed_workers,
        tracing_enabled: state.storage.is_some(),
    }))
}

pub(crate) async fn create_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let settings: LensSettings = decode(&body)?;
    let key = caller_key(&context.principal)?;
    let scope = user_scope(&key, true)?;
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
    let created = queue_job(&lens, now, state.ids.new_id(), &RunRequest::default())
        .map_err(LensMutationError::from)?;
    let repository = lens_repository(&state)?;
    Ok(Json(repository.create(created).await?))
}

pub(crate) async fn activity_available<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<ActivityAvailability>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let scope = user_scope(&caller_key(&context.principal)?, false)?;
    if state.storage.is_none() {
        return Ok(Json(ActivityAvailability::default()));
    }
    let reader = source_reader(state.storage.as_ref())?;
    Ok(Json(reader.availability(&scope).await?))
}

pub(crate) async fn list_agents<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<String>>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let scope = user_scope(&caller_key(&context.principal)?, false)?;
    if state.storage.is_none() {
        return Ok(Json(Vec::new()));
    }
    let reader = source_reader(state.storage.as_ref())?;
    Ok(Json(reader.agents(&scope).await?))
}

pub(crate) async fn read_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(lens_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let scope = user_scope(&caller_key(&context.principal)?, false)?;
    let repository = lens_repository(&state)?;
    Ok(Json(get_lens(repository, &lens_id, &scope).await?))
}

pub(crate) async fn update_lens<Db, Store>(
    State(state): State<LensRoutesState<Db, Store>>,
    Path(lens_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>>
where
    Db: Database + 'static,
    Store: Storage + Clone + Send + Sync + 'static,
{
    let Some(Extension(context)) = caller else {
        return Err(LensRouteError::Unauthenticated);
    };
    let settings: LensSettings = decode(&body)?;
    let key = caller_key(&context.principal)?;
    let scope = user_scope(&key, true)?;
    let repository = lens_repository(&state)?;
    get_lens(repository, &lens_id, &scope).await?;
    validate_model(&settings, &key, &state)?;
    let updated = repository
        .update(
            &lens_id,
            |lens| {
                let revision = lens
                    .revision
                    .checked_add(1)
                    .ok_or(LensMutationError::Revision)?;
                Ok(Lens {
                    settings: settings.clone(),
                    revision,
                    ..lens
                })
            },
            DEFAULT_ATTEMPTS,
            false,
        )
        .await
        .map_err(update_failure)?;
    Ok(Json(required(updated)?))
}

mod tests;
