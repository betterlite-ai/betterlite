use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_reset_policy::phase_failure::PhaseFailureSink;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::budget_tier_sweep::service::BudgetTierSweep;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::service::IdentityBudgetReset;
use crate::identity_budget_reset::service_hooks::PhaseSuccessSink;
use crate::postgres_store::session::PostgresStore;
use crate::team_window_reset::cache::WindowCounterCache;
use crate::team_window_reset::pagination::BudgetWindowReset;
use crate::team_window_reset::team_resets::TeamBudgetReset;
use crate::team_window_reset::window_cache_roll::WindowDurationSeconds;
use crate::window_spend::store::WindowSpendStore;

use super::settings::BudgetResetSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetPhase {
    Keys,
    Users,
    Teams,
    Tiers,
    Windows,
}

impl ResetPhase {
    pub const ALL: [Self; 5] = [
        Self::Keys,
        Self::Users,
        Self::Teams,
        Self::Tiers,
        Self::Windows,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keys => "keys",
            Self::Users => "users",
            Self::Teams => "teams",
            Self::Tiers => "tiers",
            Self::Windows => "windows",
        }
    }
}

pub trait PhaseRunner {
    type Failure: Error + Send + Sync + 'static;

    fn run(&mut self, phase: ResetPhase) -> impl Future<Output = Result<(), Self::Failure>> + Send;
}

pub struct SweepPorts<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations> {
    pub limits: ResetLimits,
    pub rollover: RolloverPolicy,
    pub default_enduser_budget_id: Option<String>,
    pub schedule: Schedule,
    pub cache: Cache,
    pub failure: Failures,
    pub success: Successes,
    pub reconnect: Option<Reconnect>,
    pub clock: Clock,
    pub durations: Durations,
}

pub struct SweepPhases<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations> {
    pub settings: BudgetResetSettings,
    pub store: PostgresStore,
    pub ports: SweepPorts<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>,
}

// The hook-carrying phase services share failure, success and the seven SweepPorts fields, so the macro writes them once.
macro_rules! sweep_service {
    (hooks $service:ident $(, $field:ident: $value:expr)* $(,)?) => {
        $service {
            store: self.store.clone(),
            limits: self.ports.limits,
            rollover: self.ports.rollover,
            schedule: &self.ports.schedule,
            cache: &self.ports.cache,
            failure: &self.ports.failure,
            success: &self.ports.success,
            reconnect: self.ports.reconnect.as_mut(),
            clock: &self.ports.clock,
            $($field: $value,)*
        }
    };
    // The window phase has no failure or success hook, and reads its spend through WindowSpendStore.
    (window) => {
        BudgetWindowReset {
            store: self.store.clone(),
            limits: self.ports.limits,
            rollover: self.ports.rollover,
            schedule: &self.ports.schedule,
            cache: &self.ports.cache,
            reconnect: self.ports.reconnect.as_mut(),
            clock: &self.ports.clock,
            window_spend: WindowSpendStore::from_store(self.store.clone()),
            durations: &self.ports.durations,
        }
    };
}

impl<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>
    SweepPhases<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>
where
    Schedule: ResetSchedule + Send + Sync,
    Cache: ResetCache + WindowCounterCache,
    Failures: PhaseFailureSink + Send + Sync,
    Successes: PhaseSuccessSink + Send + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
    Durations: WindowDurationSeconds + Send + Sync,
{
    async fn reset_keys(&mut self) {
        sweep_service!(hooks IdentityBudgetReset)
            .reset(IdentityPhase::Keys)
            .await;
    }

    async fn reset_users(&mut self) {
        sweep_service!(hooks IdentityBudgetReset)
            .reset(IdentityPhase::Users)
            .await;
    }

    async fn reset_teams(&mut self) {
        sweep_service!(hooks TeamBudgetReset).reset_teams().await;
    }

    async fn reset_tiers(&mut self) {
        let default = self.ports.default_enduser_budget_id.as_deref();
        sweep_service!(hooks BudgetTierSweep, default_enduser_budget_id: default)
            .reset()
            .await;
    }

    async fn reset_windows(&mut self) {
        sweep_service!(window).reset_windows().await;
    }
}

impl<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations> PhaseRunner
    for SweepPhases<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>
where
    Schedule: ResetSchedule + Send + Sync,
    Cache: ResetCache + WindowCounterCache,
    Failures: PhaseFailureSink + Send + Sync,
    Successes: PhaseSuccessSink + Send + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
    Durations: WindowDurationSeconds + Send + Sync,
{
    type Failure = Infallible;

    async fn run(&mut self, phase: ResetPhase) -> Result<(), Infallible> {
        match phase {
            ResetPhase::Keys => self.reset_keys().await,
            ResetPhase::Users => self.reset_users().await,
            ResetPhase::Teams => self.reset_teams().await,
            ResetPhase::Tiers => self.reset_tiers().await,
            ResetPhase::Windows => self.reset_windows().await,
        }
        Ok(())
    }
}
