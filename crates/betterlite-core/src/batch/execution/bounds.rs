use super::{BatchError, BatchLimits, Duplicate, Item};
use serde::Serialize;
use std::collections::{HashMap, hash_map::Entry};
use std::io::{self, Write};

struct ByteBudget {
    remaining: usize,
    exceeded: bool,
}
impl Write for ByteBudget {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(remaining) = self.remaining.checked_sub(buf.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("batch byte limit exceeded"));
        };
        self.remaining = remaining;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn validate<Request, Context>(
    items: &[Item<Request, Context>],
    limits: BatchLimits,
) -> Result<(), BatchError>
where
    Request: Serialize,
{
    if items.len() > limits.max_items {
        return Err(BatchError::TooManyItems {
            actual: items.len(),
            maximum: limits.max_items,
        });
    }
    if limits.concurrency == 0 {
        return Err(BatchError::ZeroConcurrency);
    }
    let mut budget = ByteBudget {
        remaining: limits.max_bytes,
        exceeded: false,
    };
    for item in items {
        if let Err(source) = serde_json::to_writer(&mut budget, item) {
            return Err(if budget.exceeded {
                BatchError::TooManyBytes {
                    maximum: limits.max_bytes,
                }
            } else {
                BatchError::Serialize(source)
            });
        }
    }
    Ok(())
}

pub(super) fn duplicates<Request, Context>(items: &[Item<Request, Context>]) -> Vec<Duplicate> {
    let mut positions = HashMap::new();
    let mut diagnostics = Vec::new();
    for (second, item) in items.iter().enumerate() {
        match positions.entry(&item.key) {
            Entry::Occupied(entry) => diagnostics.push(Duplicate {
                key: item.key.clone(),
                first: *entry.get(),
                second,
            }),
            Entry::Vacant(entry) => {
                entry.insert(second);
            }
        }
    }
    diagnostics
}
