use std::future::{Ready, ready};
use std::time::SystemTime;

use rand::Rng;

use crate::cost_routing::keys::minute_label;
use crate::latency_observations::samples::ModelGroupSamples;
use crate::latency_routing::service::{LatencyRouting, RoutingArgs, Selection};
use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;
use crate::strategy_dispatch::args::{LatencyArgs, SelectorArgs};

mod tests;

// A route carries no latency sample store, so this arm reads an empty map; once the seam exists it reads the recorded one.
pub(super) fn select<R>(
    context: &StrategyContext<'_>,
    random: &mut R,
) -> Ready<Result<usize, RoutingError>>
where
    R: Rng,
{
    // the dispatch answers every strategy its own arguments, so the declared defaults only keep this arm total
    let arguments = if let SelectorArgs::Latency(arguments) = context.dispatch.args {
        arguments
    } else {
        LatencyArgs::default()
    };
    let router = LatencyRouting {
        args: RoutingArgs {
            lowest_latency_buffer: arguments.lowest_latency_buffer,
            ttft_percentile: arguments.ttft_percentile,
        },
    };
    let samples = ModelGroupSamples::default();
    let minute = minute_label(SystemTime::now());
    let selection = Selection {
        deployments: context.deployments,
        samples: &samples,
        stream: context.route.clients.is_some_and(|clients| clients.stream),
        // B365 already counted the request; a request it could not count holds the pinned fallback of no tokens
        input_tokens: context
            .route
            .request
            .facts
            .input_token_count
            .map_or(0, |count| i64::try_from(count).unwrap_or(i64::MAX)),
        minute: &minute,
    };
    ready(
        router
            .rank(&selection)
            .map(|ranking| ranking.choose(random))
            // an index past the pool is how the pipeline reaches its no-deployment error
            .map(|choice| choice.unwrap_or(context.deployments.len())),
    )
}
