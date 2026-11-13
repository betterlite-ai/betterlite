use std::sync::{Arc, Mutex};

use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_db::lens_repository::update::LensUpdateError;
use betterlite_lens::models::decode::decode;
use betterlite_lens::models::job::{Lens, Worker};
use serde_json::Value;

use super::failure;
use super::input::{Call, Input, Transform};
use super::store::{FakeDatabase, Store, build, snapshot};
use crate::helpers::{HelperError, HelperResult};

pub(crate) async fn run(input: &Input) -> HelperResult {
    let store = Arc::new(Mutex::new(build(&input.seed, input.call.conflicts())?));
    let repository = LensRepository {
        database: FakeDatabase {
            store: Arc::clone(&store),
        },
    };
    let (result, transform_calls) = match &input.call {
        Call::Lenses => lens_list(&repository).await?,
        Call::Get { lens_id } => lens_get(&repository, lens_id).await?,
        Call::Jobs { lens_id, offset } => lens_jobs(&repository, lens_id, *offset).await?,
        Call::Job { lens_id, job_id } => lens_job(&repository, lens_id, job_id).await?,
        Call::Workers => worker_list(&repository).await?,
        Call::Worker { token_hash } => worker_get(&repository, token_hash).await?,
        _ => write_call(&repository, &input.call).await?,
    };
    Ok(lock(&store, result, transform_calls))
}

fn lock(store: &Mutex<Store>, result: Value, transform_calls: u32) -> Value {
    match store.lock() {
        Ok(guard) => snapshot(&guard, result, transform_calls),
        Err(poisoned) => snapshot(&poisoned.into_inner(), result, transform_calls),
    }
}

async fn lens_list(
    repository: &LensRepository<FakeDatabase>,
) -> Result<(Value, u32), Box<HelperError>> {
    let records = repository.lenses().await.map_err(failure)?;
    let value = serde_json::to_value(records).map_err(failure)?;
    Ok((value, 0))
}

async fn lens_get(
    repository: &LensRepository<FakeDatabase>,
    lens_id: &str,
) -> Result<(Value, u32), Box<HelperError>> {
    let found = repository.get(lens_id).await.map_err(failure)?;
    let value = serde_json::to_value(found).map_err(failure)?;
    Ok((value, 0))
}

async fn lens_jobs(
    repository: &LensRepository<FakeDatabase>,
    lens_id: &str,
    offset: i64,
) -> Result<(Value, u32), Box<HelperError>> {
    let found = repository.jobs(lens_id, offset).await.map_err(failure)?;
    let value = serde_json::to_value(found).map_err(failure)?;
    Ok((value, 0))
}

async fn lens_job(
    repository: &LensRepository<FakeDatabase>,
    lens_id: &str,
    job_id: &str,
) -> Result<(Value, u32), Box<HelperError>> {
    let found = repository.job(lens_id, job_id).await.map_err(failure)?;
    let value = serde_json::to_value(found).map_err(failure)?;
    Ok((value, 0))
}

async fn worker_list(
    repository: &LensRepository<FakeDatabase>,
) -> Result<(Value, u32), Box<HelperError>> {
    let found = repository.workers().await.map_err(failure)?;
    let value = serde_json::to_value(found).map_err(failure)?;
    Ok((value, 0))
}

async fn worker_get(
    repository: &LensRepository<FakeDatabase>,
    token_hash: &str,
) -> Result<(Value, u32), Box<HelperError>> {
    let found = repository.worker(token_hash).await.map_err(failure)?;
    let value = serde_json::to_value(found).map_err(failure)?;
    Ok((value, 0))
}

async fn write_call(
    repository: &LensRepository<FakeDatabase>,
    call: &Call,
) -> Result<(Value, u32), Box<HelperError>> {
    match call {
        Call::Create { lens } => {
            let record = decode::<Lens>(lens.to_string().as_bytes()).map_err(failure)?;
            let created = repository.create(record).await.map_err(failure)?;
            Ok((serde_json::to_value(created).map_err(failure)?, 0))
        }
        Call::Update {
            lens_id,
            attempts,
            changed_only,
            transform,
            ..
        } => update_call(repository, lens_id, *attempts, *changed_only, transform).await,
        Call::SaveWorker { worker, token_hash } => {
            let record = decode::<Worker>(worker.to_string().as_bytes()).map_err(failure)?;
            repository
                .save_worker(&record, token_hash.as_deref())
                .await
                .map_err(failure)?;
            Ok((Value::Null, 0))
        }
        Call::SetWorkerBilling { worker_id, key_id } => {
            let worker = repository
                .set_worker_billing(worker_id, key_id)
                .await
                .map_err(failure)?;
            Ok((serde_json::to_value(worker).map_err(failure)?, 0))
        }
        Call::RevokeWorker { worker_id } => {
            repository.revoke_worker(worker_id).await.map_err(failure)?;
            Ok((Value::Null, 0))
        }
        Call::Heartbeat { worker_id, now } => {
            repository
                .heartbeat(worker_id, now)
                .await
                .map_err(failure)?;
            Ok((Value::Null, 0))
        }
        _ => Err(failure("the call writes no record")),
    }
}

async fn update_call(
    repository: &LensRepository<FakeDatabase>,
    lens_id: &str,
    attempts: u32,
    changed_only: bool,
    transform: &Transform,
) -> Result<(Value, u32), Box<HelperError>> {
    let mut calls = 0_u32;
    let updated = repository
        .update(
            lens_id,
            |lens| {
                calls = calls.saturating_add(1);
                apply(lens, transform)
            },
            attempts,
            changed_only,
        )
        .await;
    match updated {
        Ok(value) => Ok((serde_json::to_value(value).map_err(failure)?, calls)),
        Err(LensUpdateError::Transform(message)) => Err(failure(message)),
        Err(LensUpdateError::Repository(error)) => Err(failure(error)),
    }
}

fn apply(mut lens: Lens, transform: &Transform) -> Result<Lens, String> {
    match transform {
        Transform::Unchanged => Ok(lens),
        Transform::SetRevision { revision } => {
            lens.revision = *revision;
            Ok(lens)
        }
        Transform::DropJob { job_id } => {
            lens.jobs.retain(|job| &job.id != job_id);
            Ok(lens)
        }
        Transform::Failing => Err("the transform failed".to_owned()),
    }
}
