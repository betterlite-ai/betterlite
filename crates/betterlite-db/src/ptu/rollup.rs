pub mod alert;
pub mod backfill;
pub mod daily;
pub mod scheduled;

mod tests;

use chrono::{Days, NaiveDate};
use deadpool_postgres::Client;
use uuid::Uuid;

use crate::database_failures::metrics::WallClock;
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;
use crate::ptu::charges::pricing::{ChargeError, PtuCharge};
use crate::ptu::store::StoreError;
use crate::ptu::store::sentinel::{ChargeWrite, write_charge_with_retry};
use crate::ptu::store::sleeper::Sleeper;

// Everything the orchestration can fail with: the session, the scan/lookup/write/prune SQL, a day's arithmetic, or a missing predecessor day.
#[derive(Debug, thiserror::Error)]
pub enum RollupError {
    #[error("the PTU rollup database session could not be opened")]
    Session(#[source] DbError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Charge(#[from] ChargeError),
    #[error("the UTC day before {day} does not exist, so the run cannot default to yesterday")]
    DayUnderflow { day: NaiveDate },
}

// prisma_client is None in the source: no pool means the run warns and reports a zero-count day instead of failing.
async fn open(store: &PostgresStore) -> Result<Option<Client>, RollupError> {
    if store.pool.is_none() {
        return Ok(None);
    }
    store
        .session()
        .await
        .map(Some)
        .map_err(RollupError::Session)
}

fn yesterday(clock: &dyn WallClock) -> Result<NaiveDate, RollupError> {
    let today = clock.now().date_naive();
    today
        .checked_sub_days(Days::new(1))
        .ok_or(RollupError::DayUnderflow { day: today })
}

// One charge through the retrying upsert: the source writes them sequentially and counts only the rows that landed.
async fn write_one<S>(
    client: &Client,
    charge: &PtuCharge,
    date: NaiveDate,
    clock: &dyn WallClock,
    sleeper: &mut S,
) -> bool
where
    S: Sleeper + ?Sized,
{
    let write = ChargeWrite {
        charge,
        date,
        id: Uuid::new_v4(),
        updated_at: clock.now(),
    };
    write_charge_with_retry(client, sleeper, &write).await
}
