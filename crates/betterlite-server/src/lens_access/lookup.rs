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
    state
        .repository
        .as_deref()
        .ok_or(LensAccessError::NoDatabase)
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
        Some(_) | None => Err(LensAccessError::LensNotFound),
    }
}

// ast-grep-ignore: no-one-line-functions - B4505 imports this fixed path to turn a lost update into a 409
pub(crate) fn required(lens: Option<Lens>) -> Result<Lens, LensAccessError> {
    lens.ok_or(LensAccessError::ConcurrentChange)
}

mod tests;
