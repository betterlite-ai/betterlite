use std::sync::{Arc, Mutex};

use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_db::lens_repository::update::LensUpdateError;
use betterlite_lens::models::decode::decode;
use betterlite_lens::models::job::{Lens, Worker};
use serde_json::Value;

use super::failure;
use super::input::{Call, Input, Transform};
use super::store::{FakeDatabase, build, snapshot};
use crate::helpers::{HelperError, HelperResult};

pub(crate) async fn run(input: &Input) -> HelperResult {
    let conflicts = match &input.call {
        Call::Update { conflicts, .. } => *conflicts,
        _ => 0,
    };
    let store = Arc::new(Mutex::new(build(&input.seed, conflicts)?));
    let repository = LensRepository {
        database: FakeDatabase {
            store: Arc::clone(&store),
        },
    };
    let (result, transform_calls) = match &input.call {
        Call::Lenses
        | Call::Get { .. }
        | Call::Jobs { .. }
        | Call::Job { .. }
        | Call::Workers
        | Call::Worker { .. } => read_call(&repository, &input.call).await?,
        _ => write_call(&repository, &input.call).await?,
    };
    let output = match store.lock() {
        Ok(guard) => snapshot(&guard, result, transform_calls),
        Err(poisoned) => snapshot(&poisoned.into_inner(), result, transform_calls),
    };
    Ok(output)
}

async fn read_call(
    repository: &LensRepository<FakeDatabase>,
    call: &Call,
) -> Result<(Value, u32), Box<HelperError>> {
    let records = match call {
        Call::Lenses => serde_json::to_value(repository.lenses().await.map_err(failure)?),
        Call::Get { lens_id } => serde_json::to_value(repository.get(lens_id).await.map_err(failure)?),
        Call::Jobs { lens_id, offset } => {
            serde_json::to_value(repository.jobs(lens_id, *offset).await.map_err(failure)?)
        }
        Call::Job { lens_id, job_id } => {
            serde_json::to_value(repository.job(lens_id, job_id).await.map_err(failure)?)
        }
        Call::Workers => serde_json::to_value(repository.workers().await.map_err(failure)?),
        Call::Worker { token_hash } => {
            serde_json::to_value(repository.worker(token_hash).await.map_err(failure)?)
        }
        _ => return Err(failure("the call reads no record")),
    };
    Ok((records.map_err(failure)?, 0))
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
            repository.heartbeat(worker_id, now).await.map_err(failure)?;
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
