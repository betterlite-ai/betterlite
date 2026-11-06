use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use super::retry::RequeuePolicy;

const PAYLOAD_TOO_LARGE: u16 = 413;

#[derive(Debug, PartialEq, Eq)]
pub enum SendFailure<C> {
    Status { status: u16 },
    Cancelled,
    Other(C),
}

#[derive(Debug, PartialEq, Eq)]
pub struct BatchCancelled<E> {
    pub undelivered: Vec<E>,
}

pub trait BatchTransport<E>: Send + Sync {
    type SendError: Send;
    type LimitError: Send;

    fn send(
        &self,
        batch: &[E],
    ) -> impl Future<Output = Result<u16, SendFailure<Self::SendError>>> + Send;

    fn exceeds_limits(&self, batch: &[E]) -> Result<bool, Self::LimitError>;
}

pub trait BatchDelivery<E>: Send + Sync {
    fn deliver(
        &self,
        batch: &[E],
    ) -> impl Future<Output = Result<Vec<E>, BatchCancelled<E>>> + Send;
}

type Delivery<'sender, E> =
    Pin<Box<dyn Future<Output = Result<Vec<E>, BatchCancelled<E>>> + Send + 'sender>>;

struct Splitter<'delivery, E, S> {
    transport: &'delivery S,
    success_status_codes: &'delivery [u16],
    policy: RequeuePolicy,
    events: PhantomData<&'delivery [E]>,
}

impl<E, S> Splitter<'_, E, S>
where
    E: Clone + Send + Sync,
    S: BatchTransport<E> + Sync,
    S::SendError: Send,
    S::LimitError: Send,
{
    fn deliver<'send>(&'send self, batch: &'send [E]) -> Delivery<'send, E> {
        Box::pin(async move {
            if batch.is_empty() {
                return Ok(Vec::new());
            }
            match self.transport.exceeds_limits(batch) {
                Ok(true) if batch.len() > 1 => return self.halve(batch).await,
                Ok(_) => {}
                Err(_) => {
                    return self
                        .halve_or_drop(
                            batch,
                            "batch delivery dropped an event that cannot be measured against the destination limits",
                        )
                        .await;
                }
            }
            match self.transport.send(batch).await {
                Ok(PAYLOAD_TOO_LARGE)
                | Err(SendFailure::Status {
                    status: PAYLOAD_TOO_LARGE,
                }) => {
                    self.halve_or_drop(
                        batch,
                        "batch delivery dropped a single event the destination rejected as too large",
                    )
                    .await
                }
                Ok(status) if self.success_status_codes.contains(&status) => Ok(Vec::new()),
                Ok(status) | Err(SendFailure::Status { status }) => {
                    Ok(self.retained(batch, status))
                }
                Err(SendFailure::Cancelled) => Err(BatchCancelled {
                    undelivered: batch.to_vec(),
                }),
                Err(SendFailure::Other(_)) => Ok(batch.to_vec()),
            }
        })
    }

    fn retained(&self, batch: &[E], status: u16) -> Vec<E> {
        if self.policy.retains(status) {
            batch.to_vec()
        } else {
            Vec::new()
        }
    }

    async fn halve_or_drop(
        &self,
        batch: &[E],
        message: &'static str,
    ) -> Result<Vec<E>, BatchCancelled<E>> {
        if batch.len() > 1 {
            return self.halve(batch).await;
        }
        tracing::error!("{message}");
        Ok(Vec::new())
    }

    async fn halve(&self, batch: &[E]) -> Result<Vec<E>, BatchCancelled<E>> {
        let midpoint = batch.len().div_euclid(2);
        let (left, right) = batch.split_at(midpoint);
        let mut undelivered = match self.deliver(left).await {
            Ok(undelivered) => undelivered,
            Err(cancelled) => {
                let mut undelivered = cancelled.undelivered;
                undelivered.extend_from_slice(right);
                return Err(BatchCancelled { undelivered });
            }
        };
        if undelivered.is_empty() {
            return self.deliver(right).await;
        }
        undelivered.extend_from_slice(right);
        Ok(undelivered)
    }
}

pub fn send_batch_with_413_split<'delivery, E, S>(
    transport: &'delivery S,
    batch: &'delivery [E],
    success_status_codes: &'delivery [u16],
    policy: RequeuePolicy,
) -> Delivery<'delivery, E>
where
    E: Clone + Send + Sync,
    S: BatchTransport<E> + Sync,
    S::SendError: Send,
    S::LimitError: Send,
{
    Box::pin(async move {
        let splitter = Splitter {
            transport,
            success_status_codes,
            policy,
            events: PhantomData,
        };
        splitter.deliver(batch).await
    })
}

mod tests;
