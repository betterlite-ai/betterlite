use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) seed: Seed,
    pub(crate) call: Call,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Seed {
    pub(crate) lenses: BTreeMap<String, LensSeed>,
    pub(crate) runs: BTreeMap<String, RunSeed>,
    pub(crate) workers: BTreeMap<String, WorkerSeed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LensSeed {
    pub(crate) version: i64,
    pub(crate) data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunSeed {
    pub(crate) lens_id: String,
    pub(crate) data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerSeed {
    pub(crate) token_hash: String,
    pub(crate) data: Value,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Call {
    Lenses,
    Get {
        lens_id: String,
    },
    Create {
        lens: Value,
    },
    Update {
        lens_id: String,
        attempts: u32,
        changed_only: bool,
        transform: Transform,
        #[serde(default)]
        conflicts: u32,
    },
    Jobs {
        lens_id: String,
        offset: i64,
    },
    Job {
        lens_id: String,
        job_id: String,
    },
    Workers,
    Worker {
        token_hash: String,
    },
    SaveWorker {
        worker: Value,
        token_hash: Option<String>,
    },
    SetWorkerBilling {
        worker_id: String,
        key_id: String,
    },
    RevokeWorker {
        worker_id: String,
    },
    Heartbeat {
        worker_id: String,
        now: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Transform {
    Unchanged,
    SetRevision {
        revision: i64,
    },
    DropJob {
        job_id: String,
    },
    Failing,
}
