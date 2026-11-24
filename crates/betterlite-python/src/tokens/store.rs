use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use betterlite_tokenizer::engine::Engine;

use super::error::TokensError;

// a Python handle is an id into this store: the wheel has no tokenizer object to hand back
static TOKENIZERS: Mutex<BTreeMap<u64, Engine>> = Mutex::new(BTreeMap::new());
// ids start at one so a handle is never the falsy zero Python callers test for
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn lock() -> Result<MutexGuard<'static, BTreeMap<u64, Engine>>, TokensError> {
    TOKENIZERS
        .lock()
        .map_err(|_poisoned| TokensError::StoreLocked)
}

pub(crate) fn insert(engine: Engine) -> Result<u64, TokensError> {
    let tokenizer = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    lock()?.insert(tokenizer, engine);
    Ok(tokenizer)
}

pub(crate) fn engine(tokenizer: u64) -> Result<Engine, TokensError> {
    let store = lock()?;
    store
        .get(&tokenizer)
        .cloned()
        .ok_or(TokensError::UnknownTokenizer { tokenizer })
}

pub(crate) fn remove(tokenizer: u64) -> Result<(), TokensError> {
    lock()?.remove(&tokenizer);
    Ok(())
}

mod tests;
