use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::scheduled_job_stagger::Settings;
use chrono::{DateTime, TimeDelta, Utc};

use super::clamp::clamped_override;
use super::identity::resolve_stagger_identity;
use super::offset::offset_seconds;
use super::staggerable::is_staggerable;
use super::window::window_for;
use super::{JobScheduler, Trigger};

// Shifts the base trigger's clock back by offset before delegating, then shifts its answer forward.
struct OffsetTrigger {
    base: Box<dyn Trigger>,
    offset: Duration,
}

impl Trigger for OffsetTrigger {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let delta = TimeDelta::from_std(self.offset).ok()?;
        let shifted_previous = match previous_fire_time {
            Some(previous) => Some(previous.checked_sub_signed(delta)?),
            None => None,
        };
        let shifted_now = now.checked_sub_signed(delta)?;
        let next = self.base.next_fire_time(shifted_previous, shifted_now)?;
        next.checked_add_signed(delta)
    }
}

fn offset_for(
    job_id: &str,
    period_seconds: Option<u32>,
    staggerable: bool,
    settings: &Settings,
    identity: &str,
) -> u32 {
    if let Some(override_value) = settings.offsets.get(job_id) {
        let non_negative = u32::try_from((*override_value).max(0)).unwrap_or(u32::MAX);
        return clamped_override(job_id, non_negative);
    }
    if !staggerable {
        return 0;
    }
    let window = window_for(job_id, period_seconds, settings.window_seconds);
    offset_seconds(job_id, identity, window)
}

// Does not check settings.enabled; the caller decides whether staggering applies at all.
#[must_use]
pub fn stagger_trigger(
    job_id: &str,
    trigger: Box<dyn Trigger>,
    period_seconds: Option<u32>,
    settings: &Settings,
    identity: &str,
) -> Box<dyn Trigger> {
    let offset = offset_for(job_id, period_seconds, true, settings, identity);
    if offset == 0 {
        trigger
    } else {
        Box::new(OffsetTrigger {
            base: trigger,
            offset: Duration::from_secs(u64::from(offset)),
        })
    }
}

fn zeroed_offsets(scheduler: &mut dyn JobScheduler) -> BTreeMap<String, u32> {
    let mut offsets = BTreeMap::new();
    for job in scheduler.jobs() {
        offsets.insert(job.id, 0);
    }
    offsets
}

pub fn apply_scheduled_job_stagger(
    scheduler: &mut dyn JobScheduler,
    settings: &Settings,
    identity: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> BTreeMap<String, u32> {
    if scheduler.running() {
        tracing::warn!(
            "scheduled job stagger skipped: the scheduler is already running, so offsets must be applied before it starts"
        );
        return zeroed_offsets(scheduler);
    }
    if !settings.enabled {
        tracing::info!("scheduled job stagger disabled via general_settings.scheduled_job_stagger");
        return zeroed_offsets(scheduler);
    }
    let resolved_identity = identity.map_or_else(
        || resolve_stagger_identity(settings.identity.as_deref(), environment),
        ToOwned::to_owned,
    );
    let mut offsets = BTreeMap::new();
    for job in scheduler.jobs() {
        let staggerable = is_staggerable(&job);
        let offset = offset_for(
            &job.id,
            job.period_seconds,
            staggerable,
            settings,
            &resolved_identity,
        );
        offsets.insert(job.id.clone(), offset);
        if offset > 0 {
            let wrapped = OffsetTrigger {
                base: job.trigger,
                offset: Duration::from_secs(u64::from(offset)),
            };
            scheduler.modify_job(&job.id, Box::new(wrapped));
        }
    }
    tracing::info!(identity = resolved_identity, offsets = ?offsets, "scheduled job stagger applied");
    offsets
}

mod tests;
