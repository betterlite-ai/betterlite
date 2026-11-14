use betterlite_lens::models::job::Lens;
use thiserror::Error;

use crate::daily_spend_sql::bind::SqlBind;
use crate::lens_repository::database::Database;
use crate::lens_repository::repository::{LensRepository, LensRepositoryError};
use crate::lens_repository::sql;

pub const DEFAULT_ATTEMPTS: u32 = 8;

#[derive(Debug, Error)]
pub enum LensUpdateError<E> {
    #[error(transparent)]
    Repository(#[from] LensRepositoryError),
    #[error("the lens update transform returned an error")]
    Transform(E),
}

impl<Db: Database> LensRepository<Db> {
    pub async fn update<E, Transform>(
        &self,
        lens_id: &str,
        mut transform: Transform,
        attempts: u32,
        changed_only: bool,
    ) -> Result<Option<Lens>, LensUpdateError<E>>
    where
        Transform: FnMut(Lens) -> Result<Lens, E>,
    {
        for _ in 0..attempts {
            let (completed, updated) = self
                .try_update(lens_id, &mut transform, changed_only)
                .await?;
            if completed {
                return Ok(updated);
            }
        }
        Ok(None)
    }

    pub(super) async fn try_update<E, Transform>(
        &self,
        lens_id: &str,
        transform: &mut Transform,
        changed_only: bool,
    ) -> Result<(bool, Option<Lens>), LensUpdateError<E>>
    where
        Transform: FnMut(Lens) -> Result<Lens, E>,
    {
        let Some(previous) = self.get(lens_id).await? else {
            return Ok((true, None));
        };
        let candidate = transform(previous.clone()).map_err(LensUpdateError::Transform)?;
        if candidate == previous {
            return Ok((true, if changed_only { None } else { Some(previous) }));
        }
        let version = previous
            .version
            .checked_add(1)
            .ok_or(LensRepositoryError::Version)?;
        let updated = Lens {
            version,
            ..candidate
        };
        let data = serde_json::to_string(&updated).map_err(LensRepositoryError::Encode)?;
        let rows = self
            .database
            .query_raw(
                sql::UPDATE_LENS,
                &[
                    SqlBind::Text(Some(data)),
                    SqlBind::Text(Some(lens_id.to_owned())),
                    SqlBind::Integer(Some(previous.version)),
                ],
            )
            .await
            .map_err(LensRepositoryError::Database)?;
        let completed = match rows.first() {
            Some(row) => {
                serde_json::from_str::<u64>(&row.data).map_err(LensRepositoryError::Count)? == 1
            }
            None => false,
        };
        Ok((completed, Some(updated)))
    }
}
