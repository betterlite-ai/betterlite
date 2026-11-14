use std::collections::{BTreeMap, BTreeSet};
use std::future::ready;
use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture};
use betterlite_lens::models::decode::decode;
use betterlite_lens::models::job::{Job, Lens, Worker};
use serde_json::{Value, json};

use super::Seed;
use super::failure;
use crate::helpers::HelperError;

mod queries;

pub(crate) struct Store {
    lenses: BTreeMap<String, StoredLens>,
    runs: BTreeMap<String, StoredRun>,
    workers: BTreeMap<String, StoredWorker>,
    conflicts: u32,
}

struct StoredLens {
    version: i64,
    data: Value,
}

struct StoredRun {
    lens_id: String,
    data: Value,
}

struct StoredWorker {
    token_hash: String,
    data: Value,
}

pub(crate) struct FakeDatabase {
    pub(crate) store: Arc<Mutex<Store>>,
}

impl Database for FakeDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        Box::pin(ready(Ok(lock_store(&self.store).query(query, args))))
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(ready(Ok(lock_store(&self.store).execute(query, args))))
    }
}

fn lock_store(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    match store.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn build(seed: &Seed, conflicts: u32) -> Result<Store, Box<HelperError>> {
    let mut store = Store {
        lenses: BTreeMap::new(),
        runs: BTreeMap::new(),
        workers: BTreeMap::new(),
        conflicts,
    };
    for (lens_id, row) in &seed.lenses {
        let lens = decode::<Lens>(row.data.to_string().as_bytes()).map_err(failure)?;
        let data = serde_json::to_value(&lens).map_err(failure)?;
        let stored = StoredLens {
            version: row.version,
            data,
        };
        store.lenses.insert(lens_id.clone(), stored);
    }
    for (job_id, row) in &seed.runs {
        let job = decode::<Job>(row.data.to_string().as_bytes()).map_err(failure)?;
        let data = serde_json::to_value(&job).map_err(failure)?;
        let lens_id = row.lens_id.clone();
        store
            .runs
            .insert(job_id.clone(), StoredRun { lens_id, data });
    }
    for (worker_id, row) in &seed.workers {
        let worker = decode::<Worker>(row.data.to_string().as_bytes()).map_err(failure)?;
        let data = serde_json::to_value(&worker).map_err(failure)?;
        let token_hash = row.token_hash.clone();
        let stored = StoredWorker { token_hash, data };
        store.workers.insert(worker_id.clone(), stored);
    }
    Ok(store)
}

pub(crate) fn snapshot(store: &Store, result: &Value, transform_calls: u32) -> Value {
    let lenses: Vec<Value> = store
        .lenses
        .iter()
        .map(|(id, row)| json!({"id": id, "version": row.version, "data": row.data}))
        .collect();
    let runs: Vec<Value> = store
        .runs
        .iter()
        .map(|(id, row)| json!({"id": id, "lens_id": row.lens_id, "data": row.data}))
        .collect();
    let workers: Vec<Value> = store
        .workers
        .iter()
        .map(|(id, row)| json!({"id": id, "token_hash": row.token_hash, "data": row.data}))
        .collect();
    json!({
        "result": result,
        "transform_calls": transform_calls,
        "lenses": lenses,
        "runs": runs,
        "workers": workers,
    })
}

fn text_argument(args: &[SqlBind], index: usize) -> Option<&str> {
    match args.get(index) {
        Some(SqlBind::Text(Some(value))) => Some(value),
        _ => None,
    }
}

fn integer_argument(args: &[SqlBind], index: usize) -> Option<i64> {
    match args.get(index) {
        Some(SqlBind::Integer(Some(value))) => Some(*value),
        _ => None,
    }
}

fn displaced_jobs(old: &Value, new: &Value) -> Vec<(String, Value)> {
    let Some(old_jobs) = old.get("jobs").and_then(Value::as_array) else {
        return Vec::new();
    };
    let retained: BTreeSet<&str> =
        new.get("jobs")
            .and_then(Value::as_array)
            .map_or_else(BTreeSet::new, |jobs| {
                jobs.iter()
                    .filter_map(|job| job.get("id").and_then(Value::as_str))
                    .collect()
            });
    old_jobs
        .iter()
        .filter_map(|job| {
            let id = job.get("id").and_then(Value::as_str)?;
            (!retained.contains(id)).then(|| (id.to_owned(), job.clone()))
        })
        .collect()
}
