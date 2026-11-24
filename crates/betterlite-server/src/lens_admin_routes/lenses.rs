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
use chrono::{DateTime, Utc};
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
        Ok(Json(state.visible_list(caller).await?))
    }

    pub(crate) async fn create_lens(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.create_from(caller, body).await?))
    }

    pub(crate) async fn activity_available(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<ActivityAvailability>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.activity(caller).await?))
    }

    pub(crate) async fn list_agents(
        State(state): State<Self>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<Vec<String>>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.agent_names(caller).await?))
    }

    pub(crate) async fn read_lens(
        State(state): State<Self>,
        Path(lens_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.read_one(caller, lens_id).await?))
    }

    pub(crate) async fn update_lens(
        State(state): State<Self>,
        Path(lens_id): Path<String>,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Json<Lens>, LensRouteError<<Store as Storage>::Error>> {
        Ok(Json(state.update_from(caller, lens_id, body).await?))
    }

    async fn visible_list(
        &self,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<LensList, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        let repository = lens_repository(self)?;
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
        Ok(LensList {
            lenses: allowed_lenses,
            workers: allowed_workers,
            tracing_enabled: self.storage.is_some(),
        })
    }

    async fn create_from(
        &self,
        caller: Option<Extension<AuthContext>>,
        body: Bytes,
    ) -> Result<Lens, LensRouteError<<Store as Storage>::Error>> {
        let (settings, key, scope) = decoded_scope::<Store>(caller, &body)?;
        validate_model(&settings, &key, self)?;
        let now = self.clock.now();
        let lens = new_lens(scope, settings, now, self.ids.new_id());
        let queued = queue_job(&lens, now, self.ids.new_id(), &RunRequest::default())
            .map_err(LensMutationError::from)?;
        let repository = lens_repository(self)?;
        Ok(repository.create(queued).await?)
    }

    async fn activity(
        &self,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<ActivityAvailability, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        if self.storage.is_none() {
            return Ok(ActivityAvailability::default());
        }
        let reader = source_reader(self.storage.as_ref())?;
        Ok(reader.availability(&scope).await?)
    }

    async fn agent_names(
        &self,
        caller: Option<Extension<AuthContext>>,
    ) -> Result<Vec<String>, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        let Some(storage) = self.storage.as_ref() else {
            return Ok(Vec::new());
        };
        let reader = source_reader(Some(storage))?;
        Ok(reader.agents(&scope).await?)
    }

    async fn read_one(
        &self,
        caller: Option<Extension<AuthContext>>,
        lens_id: String,
    ) -> Result<Lens, LensRouteError<<Store as Storage>::Error>> {
        let (_, scope) = route_key_scope::<Store>(caller, false)?;
        let repository = lens_repository(self)?;
        Ok(get_lens(repository, &lens_id, &scope).await?)
    }

    async fn update_from(
        &self,
        caller: Option<Extension<AuthContext>>,
        lens_id: String,
        body: Bytes,
    ) -> Result<Lens, LensRouteError<<Store as Storage>::Error>> {
        let (settings, key, scope) = decoded_scope::<Store>(caller, &body)?;
        let repository = lens_repository(self)?;
        get_lens(repository, &lens_id, &scope).await?;
        validate_model(&settings, &key, self)?;
        let updated = repository
            .update(
                &lens_id,
                |lens| with_settings(lens, settings.clone()),
                DEFAULT_ATTEMPTS,
                false,
            )
            .await
            .map_err(update_failure)?;
        Ok(required(updated)?)
    }
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

fn new_lens(scope: Scope, settings: LensSettings, now: DateTime<Utc>, id: String) -> Lens {
    Lens {
        id,
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
    }
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
