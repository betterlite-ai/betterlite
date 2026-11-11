use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::find_info;
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;
use betterlite_tokenizer::reservation::RequestBody;

use super::clock::{IoClock, ROUTING_ARGS_TTL_SECONDS, minute_label};
use super::counter::{CountObservation, IoCounter};
use super::error::{IoChannel, IoPreCallError, RateLimitExceeded};
use super::estimate::{InputTokenCounter, estimate_input_tokens};
use super::limits::{IoDeployment, IoLimits, MinuteKeys, minute_keys};
use super::reservation::{IoReservation, stash_reservation};

pub const DEFAULT_MAX_TOKENS: i64 = 4096;

const MAX_TOKEN_FIELDS: [&str; 3] = ["max_tokens", "max_completion_tokens", "max_output_tokens"];

pub struct IoTokenReservations {
    pub counter: Arc<dyn IoCounter>,
    pub estimator: Arc<dyn InputTokenCounter>,
    pub catalog: Arc<ModelCatalog>,
    pub clock: IoClock,
}

impl IoTokenReservations {
    pub async fn pre_call_check(
        &self,
        deployment: &IoDeployment<'_>,
        request: &mut RequestBody,
    ) -> Result<Option<IoReservation>, IoPreCallError> {
        let limits = deployment.limits;
        if limits.itpm.is_none() && limits.otpm.is_none() {
            return Ok(None);
        }
        let model = deployment.model.unwrap_or_default();
        let estimated_input = estimate_input_tokens(self.estimator.as_ref(), model, request).await;
        let max_tokens = resolve_max_tokens(request, deployment.model, &self.catalog)?;
        let minute = minute_label((self.clock)());
        let Some(keys) = minute_keys(deployment, &minute) else {
            return Ok(None);
        };
        let reservation = self
            .reserve(&keys, limits, estimated_input, max_tokens)
            .await?;
        stash_reservation(request, &reservation);
        Ok(Some(reservation))
    }

    // the pinned pre-call reserves ITPM first, then OTPM, and an OTPM failure must release the ITPM reservation
    async fn reserve(
        &self,
        keys: &MinuteKeys,
        limits: IoLimits,
        estimated_input: i64,
        max_tokens: i64,
    ) -> Result<IoReservation, RateLimitExceeded> {
        // an estimate or cap that failed still holds one token, so one request cannot fill the whole bucket
        let mut reservation = IoReservation::default();
        if let Some(limit) = limits.itpm {
            let reserved = estimated_input.max(1);
            reservation.itpm_reserved = reserved;
            reservation.itpm_key = Some(keys.itpm.clone());
            self.increment_with_rollback(&keys.itpm, reserved, limit, IoChannel::Itpm)
                .await?;
        }
        if let Some(limit) = limits.otpm {
            let reserved = if max_tokens == 0 {
                0
            } else {
                max_tokens.max(1)
            };
            reservation.otpm_reserved = reserved;
            reservation.otpm_key = Some(keys.otpm.clone());
            let increment = self
                .increment_with_rollback(&keys.otpm, reserved, limit, IoChannel::Otpm)
                .await;
            if let Err(error) = increment {
                self.release(&keys.itpm, reservation.itpm_reserved).await;
                return Err(error);
            }
        }
        Ok(reservation)
    }

    // the one rollback primitive: _sync_increment_with_rollback and _increment_with_rollback both land here
    async fn increment_with_rollback(
        &self,
        key: &str,
        value: i64,
        limit: i64,
        channel: IoChannel,
    ) -> Result<(), RateLimitExceeded> {
        if value <= 0 {
            return Ok(());
        }
        match self
            .counter
            .increment_counter(key, value, ROUTING_ARGS_TTL_SECONDS)
            .await
        {
            // the overflowing add never landed, so there is no stored increment to release
            CountObservation::Overflow => Err(RateLimitExceeded {
                channel,
                limit,
                current: i64::MAX,
            }),
            CountObservation::Counted(current) if current > limit => {
                self.release(key, value).await;
                Err(RateLimitExceeded {
                    channel,
                    limit,
                    current,
                })
            }
            CountObservation::Unavailable | CountObservation::Counted(_) => Ok(()),
        }
    }

    async fn release(&self, key: &str, value: i64) {
        if value <= 0 {
            return;
        }
        let by = value.checked_neg().unwrap_or_default();
        let _released = self
            .counter
            .increment_counter(key, by, ROUTING_ARGS_TTL_SECONDS)
            .await;
    }
}

fn resolve_max_tokens(
    request: &RequestBody,
    model: Option<&str>,
    catalog: &ModelCatalog,
) -> Result<i64, IoPreCallError> {
    for field in MAX_TOKEN_FIELDS {
        match request.get(field) {
            None | Some(Json::Null) => {}
            Some(value) => return output_cap(field, value),
        }
    }
    if let Some(model_name) = model
        && let Some(cap) = model_output_cap(catalog, model_name)
    {
        return Ok(cap);
    }
    Ok(DEFAULT_MAX_TOKENS)
}

// _model_max_output_tokens goes through get_model_info, which refuses an entry whose provider does not match
fn model_output_cap(catalog: &ModelCatalog, model: &str) -> Option<i64> {
    let query = ModelQuery {
        model,
        provider: None,
    };
    match find_info(catalog, &BEDROCK_NAMES, &query) {
        Ok(Some(resolved)) => {
            let cap = resolved
                .limits
                .max_output_tokens
                .or(resolved.limits.max_tokens)?;
            i64::try_from(cap).ok()
        }
        Ok(None) | Err(_) => None,
    }
}

// an explicit 0 is honored, a negative cap becomes 0 like max(0, int(cap)), and any other type is refused
fn output_cap(field: &'static str, value: &Json) -> Result<i64, IoPreCallError> {
    match value {
        Json::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(signed), _) => Ok(signed.max(0)),
            (None, Some(unsigned)) => Ok(i64::try_from(unsigned).unwrap_or(i64::MAX)),
            (None, None) => Err(IoPreCallError::OutputCap { field }),
        },
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(IoPreCallError::OutputCap { field })
        }
    }
}
