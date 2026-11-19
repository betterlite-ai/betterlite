use super::commit_gateway_requests::GatewayRequestsDatabase;
use super::commit_gateway_requests::commit_gateway_requests_to_db;
use super::gateway_request_accumulator::GatewayRequestAccumulator;
use super::gateway_request_redis::GatewayRequestRedisBuffer;

mod tests;

pub async fn flush_gateway_requests<Database>(
    database: &Database,
    accumulator: &GatewayRequestAccumulator,
    redis_buffer: Option<&GatewayRequestRedisBuffer>,
) where
    Database: GatewayRequestsDatabase,
{
    let snapshot = accumulator.drain();
    let Some(buffer) = redis_buffer else {
        if let Err(error) = commit_gateway_requests_to_db(database, &snapshot).await {
            accumulator.restore(&snapshot);
            tracing::warn!(
                "Gateway request tracking - failed to commit {} rows, retrying on the next flush: {error}",
                snapshot.len()
            );
        }
        return;
    };
    if let Err(error) = buffer.push(&snapshot).await {
        accumulator.restore(&snapshot);
        tracing::warn!(
            "Gateway request tracking - failed to commit {} rows, retrying on the next flush: {error}",
            snapshot.len()
        );
        return;
    }
    match buffer.commit_if_leader(database).await {
        Ok(leftover) => accumulator.restore(&leftover),
        Err(error) => tracing::warn!(
            "Gateway request tracking - leader drain failed, buffered rows stay in Redis for the next flush: {error}"
        ),
    }
}
