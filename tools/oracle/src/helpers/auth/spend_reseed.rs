mod fixtures;

use std::sync::Arc;

use betterlite_auth::budget_counter::CounterEntityType;
use betterlite_auth::spend_reseed::reseed::SpendCounterReseed;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use fixtures::{
    FakeBatch, FakeCache, FakeRows, Input, Operation, amount_json, counter, failure, timestamp,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?
            .block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let rows = Arc::new(FakeRows::build(input)?);
    let source = input.prisma.then(|| Arc::clone(&rows));
    let counter: SpendCounterReseed<FakeRows> = counter(source);
    let cache = FakeCache::build(&input.cache);
    // LiteLLM only opens a spend counter batch against a configured Redis cache
    let batch = FakeBatch::new(input.batch.clone().filter(|_| input.cache.redis));
    let entity =
        CounterEntityType::from_litellm_name(&input.entity).unwrap_or(CounterEntityType::Tag);
    let value = match &input.op {
        Operation::FromDb => counter.from_db(&input.counter_key).await.map_err(failure)?,
        Operation::EndUserFromDb => counter
            .end_user_from_db(&input.counter_key)
            .await
            .map_err(failure)?,
        Operation::IncrementInMemory => {
            let increment = Decimal::from_f64(input.increment)
                .ok_or_else(|| failure("the increment is not a decimal spend"))?;
            counter
                .increment_in_memory(&cache, &input.counter_key, increment)
                .await
                .map_err(failure)?
        }
        Operation::Coalesced => counter
            .coalesced(&cache, &batch, &input.counter_key, input.require_cache_warm)
            .await
            .map_err(failure)?,
        Operation::WindowFromTable => counter
            .window_from_table(
                &entity,
                &input.entity_id,
                input
                    .duration
                    .as_deref()
                    .ok_or_else(|| failure("window_from_table needs a duration"))?,
                timestamp(&input.expected_start)?,
            )
            .await
            .map_err(failure)?,
        Operation::WindowFromDb => counter
            .window_from_db(
                &entity,
                &input.entity_id,
                input.duration.as_deref(),
                timestamp(&input.expected_start)?,
            )
            .await
            .map_err(failure)?,
        Operation::WindowFromSpendLogs => counter
            .window_from_spend_logs(&entity, &input.entity_id, timestamp(&input.start)?)
            .await
            .map_err(failure)?,
    };
    Ok(json!({
        "value": value.map_or(Value::Null, amount_json),
        "memory_after": cache.memory_json(),
        "batch_after": batch.after(),
        "redis_sets": cache.sets_json(),
    }))
}
