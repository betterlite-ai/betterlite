use betterlite_lens::models::decode::{DecodeError, decode};
use betterlite_lens::models::job::{Job, Lens, Worker};
use thiserror::Error;

use crate::daily_spend_sql::bind::SqlBind;
use crate::lens_repository::database::{Database, DatabaseError};
use crate::lens_repository::sql;

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

impl<Db: Database> LensRepository<Db> {
    async fn one<Record, Decode>(
        &self,
        query: &str,
        args: &[SqlBind],
        decode_record: Decode,
    ) -> Result<Option<Record>, LensRepositoryError>
    where
        Decode: Fn(&str) -> Result<Record, DecodeError>,
    {
        let row = self
            .database
            .query_raw(query, args)
            .await?
            .into_iter()
            .next();
        row.map(|row| decode_record(&row.data))
            .transpose()
            .map_err(LensRepositoryError::Record)
    }

    async fn many<Record, Decode>(
        &self,
        query: &str,
        args: &[SqlBind],
        decode_record: Decode,
    ) -> Result<Vec<Record>, LensRepositoryError>
    where
        Decode: Fn(&str) -> Result<Record, DecodeError>,
    {
        let rows = self.database.query_raw(query, args).await?;
        let records: Result<Vec<_>, _> = rows.iter().map(|row| decode_record(&row.data)).collect();
        records.map_err(LensRepositoryError::Record)
    }

    pub async fn lenses(&self) -> Result<Vec<Lens>, LensRepositoryError> {
        self.many(sql::LENSES, &[], |data| decode::<Lens>(data.as_bytes()))
            .await
    }

    pub async fn get(&self, lens_id: &str) -> Result<Option<Lens>, LensRepositoryError> {
        let id = SqlBind::Text(Some(lens_id.to_owned()));
        self.one(sql::LENS, &[id], |data| decode::<Lens>(data.as_bytes()))
            .await
    }

    pub async fn create(&self, lens: Lens) -> Result<Lens, LensRepositoryError> {
        let data = serde_json::to_string(&lens).map_err(LensRepositoryError::Encode)?;
        let id = SqlBind::Text(Some(lens.id.clone()));
        self.database
            .execute_raw(sql::CREATE_LENS, &[id, SqlBind::Text(Some(data))])
            .await?;
        Ok(lens)
    }

    pub async fn jobs(&self, lens_id: &str, offset: i64) -> Result<Vec<Job>, LensRepositoryError> {
        let lens = SqlBind::Text(Some(lens_id.to_owned()));
        let args = [lens, SqlBind::Integer(Some(offset))];
        self.many(sql::JOBS, &args, |data| decode::<Job>(data.as_bytes()))
            .await
    }

    pub async fn job(
        &self,
        lens_id: &str,
        job_id: &str,
    ) -> Result<Option<Job>, LensRepositoryError> {
        let args = [
            SqlBind::Text(Some(lens_id.to_owned())),
            SqlBind::Text(Some(job_id.to_owned())),
        ];
        self.one(sql::JOB, &args, |data| decode::<Job>(data.as_bytes()))
            .await
    }

    pub async fn workers(&self) -> Result<Vec<Worker>, LensRepositoryError> {
        self.many(sql::WORKERS, &[], |data| decode::<Worker>(data.as_bytes()))
            .await
    }

    pub async fn worker(&self, token_hash: &str) -> Result<Option<Worker>, LensRepositoryError> {
        let hash = SqlBind::Text(Some(token_hash.to_owned()));
        self.one(sql::WORKER, &[hash], |data| {
            decode::<Worker>(data.as_bytes())
        })
        .await
    }

    pub async fn save_worker(
        &self,
        worker: &Worker,
        token_hash: Option<&str>,
    ) -> Result<(), LensRepositoryError> {
        let data = serde_json::to_string(worker).map_err(LensRepositoryError::Encode)?;
        let id = SqlBind::Text(Some(worker.id.clone()));
        let stored = SqlBind::Text(Some(data));
        match token_hash {
            Some(hash) => self.insert_worker(id, hash, stored).await,
            None => self.update_worker(stored, id).await,
        }
    }

    async fn insert_worker(
        &self,
        id: SqlBind,
        hash: &str,
        stored: SqlBind,
    ) -> Result<(), LensRepositoryError> {
        let hash = SqlBind::Text(Some(hash.to_owned()));
        self.database
            .execute_raw(sql::INSERT_WORKER, &[id, hash, stored])
            .await?;
        Ok(())
    }

    async fn update_worker(&self, stored: SqlBind, id: SqlBind) -> Result<(), LensRepositoryError> {
        self.database
            .execute_raw(sql::UPDATE_WORKER, &[stored, id])
            .await?;
        Ok(())
    }

    pub async fn set_worker_billing(
        &self,
        worker_id: &str,
        key_id: &str,
    ) -> Result<Option<Worker>, LensRepositoryError> {
        let args = [
            SqlBind::Text(Some(key_id.to_owned())),
            SqlBind::Text(Some(worker_id.to_owned())),
        ];
        self.one(sql::SET_WORKER_BILLING, &args, |data| {
            decode::<Worker>(data.as_bytes())
        })
        .await
    }

    pub async fn revoke_worker(&self, worker_id: &str) -> Result<(), LensRepositoryError> {
        let id = SqlBind::Text(Some(worker_id.to_owned()));
        self.database.execute_raw(sql::REVOKE_WORKER, &[id]).await?;
        Ok(())
    }

    pub async fn heartbeat(&self, worker_id: &str, now: &str) -> Result<(), LensRepositoryError> {
        let args = [
            SqlBind::Text(Some(now.to_owned())),
            SqlBind::Text(Some(worker_id.to_owned())),
        ];
        self.database.execute_raw(sql::HEARTBEAT, &args).await?;
        Ok(())
    }
}
