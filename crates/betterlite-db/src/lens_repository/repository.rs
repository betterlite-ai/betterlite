use betterlite_lens::models::decode::{DecodeError, decode};
use betterlite_lens::models::job::{Job, Lens, Worker};
use thiserror::Error;

use crate::daily_spend_sql::bind::SqlBind;
use crate::lens_repository::database::{Database, DatabaseError};
use crate::lens_repository::sql;
use crate::lens_repository::update::LensUpdateError;

#[derive(Debug, Error)]
pub enum LensRepositoryError {
    #[error("the database call failed")]
    Database(#[from] DatabaseError),
    #[error("the stored record is not valid")]
    Record(#[source] DecodeError),
    #[error("the record could not be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the lens update result is not a row count")]
    Count(#[source] serde_json::Error),
    #[error("the lens version would overflow")]
    Version,
}

pub struct LensRepository<Db> {
    pub database: Db,
}

pub type LensResult<Output> = Result<Output, LensRepositoryError>;

fn texts(values: &[&str]) -> Vec<SqlBind> {
    values
        .iter()
        .map(|value| SqlBind::Text(Some((*value).to_owned())))
        .collect()
}

impl<Db: Database> LensRepository<Db> {
    async fn records<Record, Decode>(
        &self,
        query: &str,
        args: &[SqlBind],
        decode_record: Decode,
    ) -> LensResult<Vec<Record>>
    where
        Decode: Fn(&str) -> Result<Record, DecodeError>,
    {
        let rows = self.database.query_raw(query, args).await?;
        let records: Result<Vec<_>, _> = rows.iter().map(|row| decode_record(&row.data)).collect();
        records.map_err(LensRepositoryError::Record)
    }

    pub async fn lenses(&self) -> LensResult<Vec<Lens>> {
        self.records(sql::LENSES, &[], |data| decode::<Lens>(data.as_bytes()))
            .await
    }

    pub async fn get(&self, lens_id: &str) -> LensResult<Option<Lens>> {
        let args = texts(&[lens_id]);
        let found = self.records(sql::LENS, &args, |data| decode::<Lens>(data.as_bytes()));
        Ok(found.await?.into_iter().next())
    }

    pub async fn create(&self, lens: Lens) -> LensResult<Lens> {
        let data = serde_json::to_string(&lens).map_err(LensRepositoryError::Encode)?;
        let args = texts(&[&lens.id, &data]);
        self.database.execute_raw(sql::CREATE_LENS, &args).await?;
        Ok(lens)
    }

    pub async fn jobs(&self, lens_id: &str, offset: i64) -> LensResult<Vec<Job>> {
        let lens = SqlBind::Text(Some(lens_id.to_owned()));
        let args = [lens, SqlBind::Integer(Some(offset))];
        self.records(sql::JOBS, &args, |data| decode::<Job>(data.as_bytes()))
            .await
    }

    pub async fn job(&self, lens_id: &str, job_id: &str) -> LensResult<Option<Job>> {
        let args = texts(&[lens_id, job_id]);
        let found = self.records(sql::JOB, &args, |data| decode::<Job>(data.as_bytes()));
        Ok(found.await?.into_iter().next())
    }

    pub async fn workers(&self) -> LensResult<Vec<Worker>> {
        self.records(sql::WORKERS, &[], |data| decode::<Worker>(data.as_bytes()))
            .await
    }

    pub async fn worker(&self, token_hash: &str) -> LensResult<Option<Worker>> {
        let args = texts(&[token_hash]);
        let found = self.records(sql::WORKER, &args, |data| decode::<Worker>(data.as_bytes()));
        Ok(found.await?.into_iter().next())
    }

    pub async fn save_worker(&self, worker: &Worker, token_hash: Option<&str>) -> LensResult<()> {
        let data = serde_json::to_string(worker).map_err(LensRepositoryError::Encode)?;
        let (query, args) = token_hash.map_or_else(
            || (sql::UPDATE_WORKER, texts(&[&data, &worker.id])),
            |hash| (sql::INSERT_WORKER, texts(&[&worker.id, hash, &data])),
        );
        self.database.execute_raw(query, &args).await?;
        Ok(())
    }

    pub async fn set_worker_billing(
        &self,
        worker_id: &str,
        key_id: &str,
    ) -> LensResult<Option<Worker>> {
        let args = texts(&[key_id, worker_id]);
        let rows = self.records(sql::SET_WORKER_BILLING, &args, |data| {
            decode::<Worker>(data.as_bytes())
        });
        Ok(rows.await?.into_iter().next())
    }

    pub async fn revoke_worker(&self, worker_id: &str) -> LensResult<()> {
        let args = texts(&[worker_id]);
        self.database.execute_raw(sql::REVOKE_WORKER, &args).await?;
        Ok(())
    }

    pub async fn heartbeat(&self, worker_id: &str, now: &str) -> LensResult<()> {
        let args = texts(&[now, worker_id]);
        self.database.execute_raw(sql::HEARTBEAT, &args).await?;
        Ok(())
    }

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
