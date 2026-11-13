#[path = "b4291/coordination.rs"]
mod coordination;
#[path = "b4291/disk.rs"]
mod disk;

use betterlite_context::local_ttl_cache::CacheValue;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    DiskSetAndGet {
        key: String,
        value: CacheValue,
    },
    DiskTtl {
        entries: Vec<disk::TtlEntry>,
    },
    DiskTruthyReads {
        values: Vec<CacheValue>,
    },
    DiskBatchGet {
        entries: Vec<disk::StoredEntry>,
        keys: Vec<String>,
    },
    DiskIncrement {
        key: String,
        initial: Option<CacheValue>,
        increments: Vec<i64>,
    },
    DiskDelete {
        key: String,
        value: CacheValue,
        kept: disk::StoredEntry,
    },
    CoordinatorSingleLoader {
        value: CacheValue,
        requests: usize,
    },
    CoordinatorWaiterGetsCached {
        value: CacheValue,
    },
    CoordinatorLoaderFailure {
        value: CacheValue,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::DiskSetAndGet { key, value } => disk::set_and_get(key, value),
            Input::DiskTtl { entries } => disk::ttl_expiry(entries),
            Input::DiskTruthyReads { values } => disk::truthy_reads(values),
            Input::DiskBatchGet { entries, keys } => disk::batch_get(entries, keys),
            Input::DiskIncrement {
                key,
                initial,
                increments,
            } => disk::increment(key, initial.as_ref(), increments),
            Input::DiskDelete { key, value, kept } => disk::delete(key, value, kept),
            Input::CoordinatorSingleLoader { value, requests } => coordination::run_scenario(
                value,
                coordination::Scenario::SingleLoader {
                    requests: *requests,
                },
            ),
            Input::CoordinatorWaiterGetsCached { value } => {
                coordination::run_scenario(value, coordination::Scenario::WaiterGetsCached)
            }
            Input::CoordinatorLoaderFailure { value } => {
                coordination::run_scenario(value, coordination::Scenario::LoaderFailure)
            }
        }
    }
}
