mod fixtures;

use std::sync::Arc;

use betterlite_auth::budget_counter::CounterEntityType;
use betterlite_auth::spend_reseed::reseed::SpendCounterReseed;
use betterlite_auth::spend_reseed::windows::WindowQuery;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use fixtures::{
    FakeBatch, FakeCache, FakeRows, Input, Operation, amount_json, counter, failure, timestamp,
};

type Amount = Result<Option<Decimal>, Box<HelperError>>;
type Counters = SpendCounterReseed<FakeRows>;

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
    let source = input.prisma.0.then(|| Arc::clone(&rows));
    let reseed: Counters = counter(source);
    let cache = FakeCache::build(&input.cache);
    // LiteLLM only opens a spend counter batch against a configured Redis cache
    let batch = FakeBatch::new(input.batch.clone().filter(|_| input.cache.redis));
    let value = match &input.op {
        Operation::FromDb => reseed.from_db(&input.counter_key).await.map_err(failure)?,
        Operation::EndUserFromDb => reseed
            .end_user_from_db(&input.counter_key)
            .await
            .map_err(failure)?,
        Operation::IncrementInMemory => increment(input, &reseed, &cache).await?,
        Operation::Coalesced => {
            let warm = input.require_cache_warm.0;
            reseed
                .coalesced(&cache, &batch, &input.counter_key, warm)
                .await
                .map_err(failure)?
        }
        Operation::WindowFromTable => window(input, &reseed, WindowOp::Table).await?,
        Operation::WindowFromDb => window(input, &reseed, WindowOp::Db).await?,
        Operation::WindowFromSpendLogs => window(input, &reseed, WindowOp::Logs).await?,
    };
    Ok(json!({
        "value": value.map_or(Value::Null, amount_json),
        "memory_after": cache.memory_json(),
        "batch_after": batch.after(),
        "redis_sets": cache.sets_json(),
    }))
}

async fn increment(input: &Input, counter: &Counters, cache: &FakeCache) -> Amount {
    let increment = Decimal::from_f64(input.increment)
        .ok_or_else(|| failure("the increment is not a decimal spend"))?;
    counter
        .increment_in_memory(cache, &input.counter_key, increment)
        .await
        .map_err(failure)
}

fn counter_entity(input: &Input) -> CounterEntityType {
    CounterEntityType::from_litellm_name(&input.entity).unwrap_or(CounterEntityType::Tag)
}

#[derive(Clone, Copy)]
enum WindowOp {
    Table,
    Db,
    Logs,
}

fn window_query<'input>(
    input: &'input Input,
    entity: &'input CounterEntityType,
) -> Result<WindowQuery<'input>, Box<HelperError>> {
    Ok(WindowQuery {
        entity_type: entity,
        entity_id: &input.entity_id,
        start: timestamp(&input.start)?,
    })
}

async fn window(input: &Input, counter: &Counters, op: WindowOp) -> Amount {
    let entity = counter_entity(input);
    match op {
        WindowOp::Table => {
            let Some(duration) = input.duration.as_deref() else {
                return Err(failure("window_from_table needs a duration"));
            };
            let expected = timestamp(&input.expected_start)?;
            let query = window_query(input, &entity)?;
            counter
                .window_from_table(&query, duration, expected)
                .await
                .map_err(failure)
        }
        WindowOp::Db => {
            let duration = input.duration.as_deref();
            let expected = timestamp(&input.expected_start)?;
            counter
                .window_from_db(&entity, &input.entity_id, duration, expected)
                .await
                .map_err(failure)
        }
        WindowOp::Logs => counter
            .window_from_spend_logs(&window_query(input, &entity)?)
            .await
            .map_err(failure),
    }
}
