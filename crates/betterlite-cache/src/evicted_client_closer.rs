use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use tokio::runtime::{Handle, Id as RuntimeId};
use tokio::time::timeout;

pub const CLOSE_GRACE_SECONDS: f64 = 900.0;
pub const MAX_PENDING_CLOSES: usize = 10_000;
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub type EventLoopId = RuntimeId;

#[must_use]
pub fn current_event_loop_id() -> Option<EventLoopId> {
    match Handle::try_current() {
        Ok(handle) => Some(handle.id()),
        Err(_no_runtime) => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseMethod {
    Sync,
    Async,
}

// the typed stand-in for LiteLLM's attribute probing of aclose/close and transport connection state
#[async_trait]
pub trait EvictableClient: Send + Sync + 'static {
    fn close_method(&self) -> Option<CloseMethod>;

    fn has_connection_in_flight(&self) -> bool {
        false
    }

    fn close(&self) {}

    async fn close_async(&self) {}
}

struct PendingClose<Client: EvictableClient> {
    client: Weak<Client>,
    event_loop: Option<EventLoopId>,
    needs_loop: bool,
    close_after: f64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum BucketKey {
    ClosableAnywhere,
    ClosableOnAnyLoop,
    EventLoop(EventLoopId),
}

struct Queue<Client: EvictableClient> {
    buckets: HashMap<BucketKey, VecDeque<PendingClose<Client>>>,
    pending: usize,
}

pub struct EvictedClientCloser<Client: EvictableClient> {
    grace_seconds: f64,
    max_pending: usize,
    close_timeout: Duration,
    clock: Clock,
    owned: Mutex<Vec<Weak<Client>>>,
    queue: Mutex<Queue<Client>>,
}

fn bucket_key<Client>(pending: &PendingClose<Client>) -> BucketKey
where
    Client: EvictableClient,
{
    if !pending.needs_loop {
        return BucketKey::ClosableAnywhere;
    }
    pending
        .event_loop
        .map_or(BucketKey::ClosableOnAnyLoop, BucketKey::EventLoop)
}

fn bucket_order(event_loop: Option<EventLoopId>) -> Vec<BucketKey> {
    let mut keys = Vec::with_capacity(3);
    keys.push(BucketKey::ClosableAnywhere);
    if let Some(loop_id) = event_loop {
        keys.push(BucketKey::ClosableOnAnyLoop);
        keys.push(BucketKey::EventLoop(loop_id));
    }
    keys
}

fn push_pending<Client>(queue: &mut Queue<Client>, pending: PendingClose<Client>)
where
    Client: EvictableClient,
{
    let bucket = queue.buckets.entry(bucket_key(&pending)).or_default();
    while bucket
        .front()
        .is_some_and(|front| front.client.strong_count() == 0)
    {
        bucket.pop_front();
        queue.pending = queue.pending.saturating_sub(1);
    }
    bucket.push_back(pending);
    queue.pending = queue.pending.saturating_add(1);
}

fn take_bucket<Client>(
    buckets: &mut HashMap<BucketKey, VecDeque<PendingClose<Client>>>,
    pending: &mut usize,
    key: &BucketKey,
    now: f64,
    due: &mut Vec<PendingClose<Client>>,
) where
    Client: EvictableClient,
{
    let Some(bucket) = buckets.get_mut(key) else {
        return;
    };
    let mut drained = drain_bucket(bucket, now);
    *pending = pending.saturating_sub(drained.len());
    if bucket.is_empty() {
        buckets.remove(key);
    }
    due.append(&mut drained);
}

fn drain_bucket<Client>(
    bucket: &mut VecDeque<PendingClose<Client>>,
    now: f64,
) -> Vec<PendingClose<Client>>
where
    Client: EvictableClient,
{
    let mut due = Vec::new();
    while bucket.front().is_some_and(|front| front.close_after <= now) {
        let Some(pending) = bucket.pop_front() else {
            break;
        };
        due.push(pending);
    }
    due
}

impl<Client: EvictableClient> EvictedClientCloser<Client> {
    #[must_use]
    pub fn new(clock: Clock) -> Self {
        Self {
            grace_seconds: CLOSE_GRACE_SECONDS,
            max_pending: MAX_PENDING_CLOSES,
            close_timeout: CLOSE_TIMEOUT,
            clock,
            owned: Mutex::new(Vec::new()),
            queue: Mutex::new(Queue {
                buckets: HashMap::new(),
                pending: 0,
            }),
        }
    }

    #[must_use]
    pub fn with_limits(
        grace_seconds: f64,
        max_pending: usize,
        close_timeout: Duration,
        clock: Clock,
    ) -> Self {
        Self {
            grace_seconds,
            max_pending,
            close_timeout,
            clock,
            owned: Mutex::new(Vec::new()),
            queue: Mutex::new(Queue {
                buckets: HashMap::new(),
                pending: 0,
            }),
        }
    }

    pub fn mark_owned(&self, client: &Arc<Client>) {
        let mut owned = self.owned.lock().unwrap_or_else(PoisonError::into_inner);
        owned.retain(|entry| entry.strong_count() > 0);
        owned.push(Arc::downgrade(client));
    }

    fn is_owned(&self, client: &Weak<Client>) -> bool {
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|entry| Weak::ptr_eq(entry, client))
    }

    pub fn schedule(&self, client: &Arc<Client>) {
        let client_ref = Arc::downgrade(client);
        if !self.is_owned(&client_ref) {
            return;
        }
        let Some(close_method) = client.close_method() else {
            return;
        };
        let event_loop = current_event_loop_id();
        let close_after = (self.clock)().mul_add(1.0_f64, self.grace_seconds);
        let pending = PendingClose {
            client: client_ref,
            event_loop,
            needs_loop: close_method == CloseMethod::Async,
            close_after,
        };
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        if queue.pending >= self.max_pending {
            return;
        }
        push_pending(&mut queue, pending);
        drop(queue);
    }

    // closes the due clients whose close is synchronous and returns the due clients that need an await
    fn drain_due(&self, event_loop: Option<EventLoopId>, now: f64) -> Vec<Arc<Client>> {
        let mut async_due = Vec::new();
        for pending in self.take_due(event_loop, now) {
            let Some(client) = pending.client.upgrade() else {
                continue;
            };
            if client.has_connection_in_flight() {
                self.requeue(pending, now);
                continue;
            }
            if pending.needs_loop {
                async_due.push(client);
            } else {
                client.close();
            }
        }
        async_due
    }

    // no running loop means only the bucket of clients that close synchronously is taken
    pub fn reap(&self) {
        if self.pending_count() == 0 {
            return;
        }
        let now = (self.clock)();
        self.drain_due(None, now);
    }

    pub async fn reap_async(&self) {
        if self.pending_count() == 0 {
            return;
        }
        let now = (self.clock)();
        for client in self.drain_due(current_event_loop_id(), now) {
            if timeout(self.close_timeout, client.close_async())
                .await
                .is_err()
            {
                tracing::debug!("an evicted client close exceeded the close timeout");
            }
        }
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
    }

    fn take_due(&self, event_loop: Option<EventLoopId>, now: f64) -> Vec<PendingClose<Client>> {
        let mut guard = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let queue = &mut *guard;
        let mut due = Vec::new();
        for key in bucket_order(event_loop) {
            take_bucket(&mut queue.buckets, &mut queue.pending, &key, now, &mut due);
        }
        drop(guard);
        due
    }

    fn requeue(&self, pending: PendingClose<Client>, now: f64) {
        let requeued = PendingClose {
            client: pending.client,
            event_loop: pending.event_loop,
            needs_loop: pending.needs_loop,
            close_after: now.mul_add(1.0_f64, self.grace_seconds),
        };
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        push_pending(&mut queue, requeued);
        drop(queue);
    }
}

#[path = "evicted_client_closer/tests.rs"]
mod tests;
