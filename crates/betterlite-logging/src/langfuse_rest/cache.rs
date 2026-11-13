use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::records::Prompt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PromptKey {
    pub name: String,
    pub version: Option<i64>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedPrompt {
    pub prompt: Prompt,
    pub fetched_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CachedLookup {
    Fresh(Prompt),
    Stale(Prompt),
}

#[derive(Debug, Default)]
pub struct PromptCache {
    pub entries: HashMap<PromptKey, CachedPrompt>,
    pub refreshing: HashSet<PromptKey>,
}

impl PromptCache {
    #[must_use]
    pub fn lookup(&self, key: &PromptKey, ttl: Duration, now: Instant) -> Option<CachedLookup> {
        let entry = self.entries.get(key)?;
        let elapsed = now.duration_since(entry.fetched_at);
        if elapsed >= ttl {
            return Some(CachedLookup::Stale(entry.prompt.clone()));
        }
        Some(CachedLookup::Fresh(entry.prompt.clone()))
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
