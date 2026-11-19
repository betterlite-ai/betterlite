use std::time::UNIX_EPOCH;

use crate::limiter_records::{CacheCounterValue, ParallelRequestGauge, RateLimitResponse};
use crate::rate_limit_window_store::CounterValues;
use crate::rate_limit_window_store::batch_get::batch_get_counter_values;
use crate::rate_limit_window_store::in_memory_window::in_memory_cache_sliding_window;
use crate::rate_limit_window_store::read_fallback::read_counter_snapshot;

use super::AdmissionRequest;
use super::OK_CODE;
use super::OVER_LIMIT_CODE;
use super::WindowedAdmission;
use super::WindowedAdmissionError;
use super::batch_script::BatchRateLimiterScript as _;
use super::collect::CollectedWindowedKeys;
use super::over_limit::is_cache_list_over_limit;

// the workspace denies multiple_inherent_impl, so the windowed steps live on this trait
pub(super) trait WindowedResponseFlow {
    async fn windowed_response(
        &self,
        collected: &CollectedWindowedKeys,
        read_only: bool,
        now_int: i64,
    ) -> Result<RateLimitResponse, WindowedAdmissionError>;

    async fn windowed_values(
        &self,
        keys_to_fetch: &[String],
        read_only: bool,
        now_int: i64,
    ) -> Result<CounterValues, WindowedAdmissionError>;

    async fn write_back(
        &self,
        keys_to_fetch: &[String],
        values: &[CacheCounterValue],
    ) -> Result<(), WindowedAdmissionError>;

    async fn gauge_response(
        &self,
        gauges: &[ParallelRequestGauge],
        request: &AdmissionRequest,
        windowed: &RateLimitResponse,
    ) -> Result<RateLimitResponse, WindowedAdmissionError>;

    fn now_int(&self) -> Result<i64, WindowedAdmissionError>;
}

impl WindowedResponseFlow for WindowedAdmission {
    async fn windowed_response(
        &self,
        collected: &CollectedWindowedKeys,
        read_only: bool,
        now_int: i64,
    ) -> Result<RateLimitResponse, WindowedAdmissionError> {
        if collected.keys_to_fetch.is_empty() {
            return Ok(RateLimitResponse {
                overall_code: OK_CODE.to_owned(),
                statuses: Vec::new(),
                reservation_windows: None,
            });
        }
        let local = batch_get_counter_values(&self.windows, &collected.keys_to_fetch, true).await;
        if let Some(values) = local {
            let response = is_cache_list_over_limit(
                &collected.keys_to_fetch,
                &values,
                &collected.key_metadata,
                read_only,
            )?;
            if response.overall_code == OVER_LIMIT_CODE {
                return Ok(response);
            }
        }
        let values = self
            .windowed_values(&collected.keys_to_fetch, read_only, now_int)
            .await?;
        is_cache_list_over_limit(
            &collected.keys_to_fetch,
            &values,
            &collected.key_metadata,
            read_only,
        )
    }

    async fn windowed_values(
        &self,
        keys_to_fetch: &[String],
        read_only: bool,
        now_int: i64,
    ) -> Result<CounterValues, WindowedAdmissionError> {
        if read_only {
            let values =
                read_counter_snapshot(&self.windows, &self.fail_closed, keys_to_fetch).await?;
            return Ok(values.unwrap_or_else(|| synthesized_values(keys_to_fetch, now_int)));
        }
        if self.windows.rate_limiter_script.is_none() {
            return in_memory_cache_sliding_window(
                &self.windows,
                keys_to_fetch,
                now_int,
                self.window_size,
            )
            .await
            .map_err(WindowedAdmissionError::from);
        }
        let values = self
            .execute_redis_batch_rate_limiter_script(keys_to_fetch, now_int)
            .await?;
        self.write_back(keys_to_fetch, &values).await?;
        Ok(values.into_iter().map(Some).collect())
    }

    async fn write_back(
        &self,
        keys_to_fetch: &[String],
        values: &[CacheCounterValue],
    ) -> Result<(), WindowedAdmissionError> {
        if keys_to_fetch.len() != values.len() || !values.len().is_multiple_of(2) {
            return Err(pair_error(keys_to_fetch, values));
        }
        let pairs = keys_to_fetch
            .as_chunks::<2>()
            .0
            .iter()
            .zip(values.as_chunks::<2>().0.iter());
        for (pair_keys, pair_values) in pairs {
            let [window_key, counter_key] = pair_keys;
            let [window_value, counter_value] = pair_values;
            let cache = self.windows.cache.as_ref();
            cache
                .set_cache(counter_key, counter_value.clone(), self.window_size, true)
                .await;
            cache
                .set_cache(window_key, window_value.clone(), self.window_size, true)
                .await;
        }
        Ok(())
    }

    async fn gauge_response(
        &self,
        gauges: &[ParallelRequestGauge],
        request: &AdmissionRequest,
        windowed: &RateLimitResponse,
    ) -> Result<RateLimitResponse, WindowedAdmissionError> {
        let slot_id = request
            .parallel_slot_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map_or_else(|| self.slot_ids.next_slot_id(), str::to_owned);
        let response = self
            .gauges
            .check_parallel_request_gauges(gauges, &slot_id, request.read_only)
            .await?;
        let mut statuses = windowed.statuses.clone();
        statuses.extend(response.statuses);
        Ok(RateLimitResponse {
            overall_code: response.overall_code,
            statuses,
            reservation_windows: None,
        })
    }

    fn now_int(&self) -> Result<i64, WindowedAdmissionError> {
        let elapsed = self
            .clock
            .current_time()
            .duration_since(UNIX_EPOCH)
            .map_err(WindowedAdmissionError::ClockBeforeEpoch)?;
        let seconds = elapsed.as_secs();
        i64::try_from(seconds).map_err(|source| WindowedAdmissionError::ClockOverflow {
            value: seconds,
            source,
        })
    }
}

fn synthesized_values(keys_to_fetch: &[String], now_int: i64) -> CounterValues {
    keys_to_fetch
        .iter()
        .map(|key| {
            let value = if key.ends_with(":window") {
                CacheCounterValue::Text(now_int.to_string())
            } else {
                CacheCounterValue::Int(0)
            };
            Some(value)
        })
        .collect()
}

const fn pair_error(
    keys_to_fetch: &[String],
    values: &[CacheCounterValue],
) -> WindowedAdmissionError {
    WindowedAdmissionError::WindowPairCount {
        keys: keys_to_fetch.len(),
        values: values.len(),
    }
}

mod tests;
