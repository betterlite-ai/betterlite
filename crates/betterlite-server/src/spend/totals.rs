mod access;
mod handlers;
pub mod prometheus;
pub mod query;
mod responses;
pub mod rows;
pub mod state;
pub mod store;
mod teams;
mod tests;

use axum::Router;
use axum::extract::Extension;
use axum::routing::get;

use self::handlers::{Report, respond};
use self::prometheus::DailySpendSource;
use self::state::TotalsState;

pub fn routes<S>(state: TotalsState<S>) -> Router
where
    S: DailySpendSource + 'static,
{
    let reports = [
        ("/global/spend/logs", Report::Logs),
        ("/global/spend", Report::Spend),
        ("/global/spend/keys", Report::Keys),
        ("/global/spend/models", Report::Models),
        ("/global/spend/teams", Report::Teams),
    ];
    let mut router = Router::new();
    for (path, report) in reports {
        router = router.route(path, get(respond::<S>).layer(Extension(report)));
    }
    router.with_state(state)
}
