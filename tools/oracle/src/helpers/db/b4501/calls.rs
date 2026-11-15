use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_db::lens_repository::update::LensUpdateError;
use betterlite_lens::models::decode::decode;
use betterlite_lens::models::job::{Lens, Worker};
use serde_json::Value;

use super::store::{FakeDatabase, build, snapshot};
use super::{Call, Input, Transform, failure};
use crate::helpers::{HelperError, HelperResult};

pub(crate) async fn run(input: &Input) -> HelperResult {
    let store = Arc::new(Mutex::new(build(&input.seed, input.call.conflicts())?));
    let repository = LensRepository {
        database: FakeDatabase {
            store: Arc::clone(&store),
        },
    };
    let (result, transform_calls) = match &input.call {
        Call::Update {
            lens_id,
            attempts,
            changed_only,
            transform,
            ..
        } => update_call(&repository, lens_id, *attempts, *changed_only, transform).await?,
        Call::Jobs { lens_id, offset } => {
            let found = repository.jobs(lens_id, *offset).await.map_err(failure)?;
            (serde_json::to_value(found).map_err(failure)?, 0)
        }
        Call::Job { lens_id, job_id } => {
            let found = repository.job(lens_id, job_id).await.map_err(failure)?;
            (serde_json::to_value(found).map_err(failure)?, 0)
        }
        Call::SaveWorker { .. }
        | Call::SetWorkerBilling { .. }
        | Call::RevokeWorker { .. }
        | Call::Heartbeat { .. } => (worker_call(&repository, &input.call).await?, 0),
    };
    let guard = store.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(snapshot(&guard, &result, transform_calls))
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
                Ok::<Lens, Infallible>(apply(lens, transform))
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

async fn worker_call(
    repository: &LensRepository<FakeDatabase>,
    call: &Call,
) -> Result<Value, Box<HelperError>> {
    match call {
        Call::SaveWorker { worker, token_hash } => {
            let record = decode::<Worker>(worker.to_string().as_bytes()).map_err(failure)?;
            let saved = repository.save_worker(&record, token_hash.as_deref());
            saved.await.map_err(failure)?;
            Ok(Value::Null)
        }
        Call::SetWorkerBilling { worker_id, key_id } => {
            let worker = repository.set_worker_billing(worker_id, key_id);
            serde_json::to_value(worker.await.map_err(failure)?).map_err(failure)
        }
        Call::RevokeWorker { worker_id } => {
            repository.revoke_worker(worker_id).await.map_err(failure)?;
            Ok(Value::Null)
        }
        Call::Heartbeat { worker_id, now } => {
            let beat = repository.heartbeat(worker_id, now);
            beat.await.map_err(failure)?;
            Ok(Value::Null)
        }
        Call::Update { .. } | Call::Jobs { .. } | Call::Job { .. } => {
            Err(failure("the call is not a worker operation"))
        }
    }
}

fn apply(mut lens: Lens, transform: &Transform) -> Lens {
    match transform {
        Transform::Unchanged => {}
        Transform::SetRevision { revision } => lens.revision = *revision,
        Transform::DropJob { job_id } => lens.jobs.retain(|job| job.id != *job_id),
    }
    lens
}
