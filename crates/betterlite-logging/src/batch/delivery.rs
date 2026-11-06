use std::fmt::Display;
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

#[derive(Debug, Clone, Copy)]
pub struct DeliverySettings<'text> {
    pub success_status_codes: &'text [u16],
    pub policy: RequeuePolicy,
    pub integration_name: &'text str,
    pub detail: &'text str,
    pub drop_error_message: &'text str,
}

pub trait BatchTransport<E>: Send + Sync {
    type SendError: Send + Display;
    type LimitError: Send + Display;

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
    settings: DeliverySettings<'delivery>,
    events: PhantomData<&'delivery [E]>,
}

impl<E, S> Splitter<'_, E, S>
where
    E: Clone + Send + Sync,
    S: BatchTransport<E> + Sync,
{
    fn deliver<'send>(&'send self, batch: &'send [E]) -> Delivery<'send, E> {
        Box::pin(async move {
            if batch.is_empty() {
                return Ok(Vec::new());
            }
            if let Some(outcome) = self.split_when_oversized(batch).await {
                return outcome;
            }
            match self.transport.send(batch).await {
                Ok(PAYLOAD_TOO_LARGE)
                | Err(SendFailure::Status {
                    status: PAYLOAD_TOO_LARGE,
                }) => {
                    self.halve_or_drop(batch, self.settings.drop_error_message)
                        .await
                }
                Ok(status) if self.settings.success_status_codes.contains(&status) => {
                    Ok(Vec::new())
                }
                Ok(status) | Err(SendFailure::Status { status }) => {
                    Ok(self.retained(batch, status))
                }
                Err(SendFailure::Cancelled) => Err(BatchCancelled {
                    undelivered: batch.to_vec(),
                }),
                Err(SendFailure::Other(error)) => {
                    tracing::error!(
                        integration = self.settings.integration_name,
                        %error,
                        records = batch.len(),
                        detail = self.settings.detail,
                        "the batch delivery send failed; the whole batch is requeued"
                    );
                    Ok(batch.to_vec())
                }
            }
        })
    }

    async fn split_when_oversized(&self, batch: &[E]) -> Option<Result<Vec<E>, BatchCancelled<E>>> {
        match self.transport.exceeds_limits(batch) {
            Ok(true) if batch.len() > 1 => Some(self.halve(batch).await),
            Ok(_) => None,
            Err(error) => {
                tracing::error!(
                    integration = self.settings.integration_name,
                    %error,
                    records = batch.len(),
                    detail = self.settings.detail,
                    "the integration dropped a record that cannot be serialized"
                );
                Some(
                    self.halve_or_drop(
                        batch,
                        "batch delivery dropped a record that cannot be serialized",
                    )
                    .await,
                )
            }
        }
    }

    fn retained(&self, batch: &[E], status: u16) -> Vec<E> {
        self.settings.policy.retained(
            batch,
            status,
            self.settings.integration_name,
            self.settings.detail,
        )
    }

    async fn halve_or_drop(&self, batch: &[E], message: &str) -> Result<Vec<E>, BatchCancelled<E>> {
        if batch.len() > 1 {
            return self.halve(batch).await;
        }
        tracing::error!(
            integration = self.settings.integration_name,
            records = batch.len(),
            detail = self.settings.detail,
            "{message}"
        );
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
    settings: DeliverySettings<'delivery>,
) -> Delivery<'delivery, E>
where
    E: Clone + Send + Sync,
    S: BatchTransport<E> + Sync,
{
    Box::pin(async move {
        let splitter = Splitter {
            transport,
            settings,
            events: PhantomData,
        };
        splitter.deliver(batch).await
    })
}

mod tests;
