use std::future::Future;
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

type Delivery<'delivery, E> =
    Pin<Box<dyn Future<Output = Result<Vec<E>, BatchCancelled<E>>> + Send + 'delivery>>;

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
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        match transport.exceeds_limits(batch) {
            Ok(true) if batch.len() > 1 => {
                return halve(transport, batch, success_status_codes, policy).await;
            }
            Ok(_) => {}
            Err(_) => {
                if batch.len() > 1 {
                    return halve(transport, batch, success_status_codes, policy).await;
                }
                tracing::error!(
                    "batch delivery dropped an event that cannot be measured against the destination limits"
                );
                return Ok(Vec::new());
            }
        }
        match transport.send(batch).await {
            Ok(PAYLOAD_TOO_LARGE)
            | Err(SendFailure::Status {
                status: PAYLOAD_TOO_LARGE,
            }) => handle_payload_too_large(transport, batch, success_status_codes, policy).await,
            Ok(status) if success_status_codes.contains(&status) => Ok(Vec::new()),
            Ok(status) | Err(SendFailure::Status { status }) => Ok(policy.retained(batch, status)),
            Err(SendFailure::Cancelled) => Err(BatchCancelled {
                undelivered: batch.to_vec(),
            }),
            Err(SendFailure::Other(_)) => Ok(batch.to_vec()),
        }
    })
}

async fn halve<E, S>(
    transport: &S,
    batch: &[E],
    success_status_codes: &[u16],
    policy: RequeuePolicy,
) -> Result<Vec<E>, BatchCancelled<E>>
where
    E: Clone + Send + Sync,
    S: BatchTransport<E> + Sync,
    S::SendError: Send,
    S::LimitError: Send,
{
    let midpoint = batch.len().div_euclid(2);
    let (left, right) = batch.split_at(midpoint);
    let mut undelivered =
        match send_batch_with_413_split(transport, left, success_status_codes, policy).await {
            Ok(undelivered) => undelivered,
            Err(cancelled) => {
                let mut undelivered = cancelled.undelivered;
                undelivered.extend_from_slice(right);
                return Err(BatchCancelled { undelivered });
            }
        };
    if undelivered.is_empty() {
        return send_batch_with_413_split(transport, right, success_status_codes, policy).await;
    }
    undelivered.extend_from_slice(right);
    Ok(undelivered)
}

async fn handle_payload_too_large<E, S>(
    transport: &S,
    batch: &[E],
    success_status_codes: &[u16],
    policy: RequeuePolicy,
) -> Result<Vec<E>, BatchCancelled<E>>
where
    E: Clone + Send + Sync,
    S: BatchTransport<E> + Sync,
    S::SendError: Send,
    S::LimitError: Send,
{
    if batch.len() > 1 {
        return halve(transport, batch, success_status_codes, policy).await;
    }
    tracing::error!("batch delivery dropped a single event the destination rejected as too large");
    Ok(Vec::new())
}

mod tests;
