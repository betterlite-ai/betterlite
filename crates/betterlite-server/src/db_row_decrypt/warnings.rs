use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use indexmap::IndexMap;

// _log_ignored_cost_map_copy is lru_cache(maxsize=4096)
const MAX_COST_MAP_COPY_WARNINGS: usize = 4096;

// The once-per-process warnings the row helpers emit: the MCP stdio flag and each echoed cost map copy.
#[derive(Debug, Default)]
pub struct Warnings {
    stdio_flag: AtomicBool,
    cost_map_copies: Mutex<IndexMap<(String, Vec<String>), ()>>,
}

impl Warnings {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - the swap is the whole once-only check for the stdio flag
    pub(crate) fn first_stdio_flag(&self) -> bool {
        !self.stdio_flag.swap(true, Ordering::SeqCst)
    }

    // true the first time this deployment's echoed field set is seen; an lru cache keeps the newest 4096
    #[must_use]
    pub(crate) fn first_cost_map_copy(&self, model_id: &str, fields: &[String]) -> bool {
        let key = (model_id.to_owned(), fields.to_vec());
        let mut seen = self
            .cost_map_copies
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if seen.shift_remove(&key).is_some() {
            seen.insert(key, ());
            return false;
        }
        seen.insert(key, ());
        while seen.len() > MAX_COST_MAP_COPY_WARNINGS {
            seen.shift_remove_index(0);
        }
        drop(seen);
        true
    }
}
