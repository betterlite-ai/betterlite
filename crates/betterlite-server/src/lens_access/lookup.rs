use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::Lens;
use betterlite_lens::models::settings::Scope;
use betterlite_lens::state::transitions::can_access;

use crate::lens_access::error::LensAccessError;
use crate::lens_access::state::LensRoutesState;

pub(crate) fn lens_repository<Db, Store>(
    state: &LensRoutesState<Db, Store>,
) -> Result<&LensRepository<Db>, LensAccessError> {
    match state.repository.as_deref() {
        Some(repository) => Ok(repository),
        None => Err(LensAccessError::NoDatabase),
    }
}

pub(crate) async fn get_lens<Db>(
    repository: &LensRepository<Db>,
    lens_id: &str,
    scope: &Scope,
) -> Result<Lens, LensAccessError>
where
    Db: Database,
{
    let found = repository
        .get(lens_id)
        .await
        .map_err(LensAccessError::Repository)?;
    match found {
        Some(lens) if can_access(scope, &lens.scope) => Ok(lens),
        Some(_lens) | None => Err(LensAccessError::LensNotFound),
    }
}

pub(crate) fn required(lens: Option<Lens>) -> Result<Lens, LensAccessError> {
    match lens {
        Some(lens) => Ok(lens),
        None => Err(LensAccessError::ConcurrentChange),
    }
}

mod tests;
