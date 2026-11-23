use std::sync::PoisonError;

use async_trait::async_trait;
use betterlite_server::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use betterlite_server::shadow_eval_lifecycle::store::{
    ClaimedTarget, InsertRejection, NewLeg, ShadowLifecycleStore, TargetRows,
};
use betterlite_server::shadow_eval_models::ShadowEvalTargetType;
use betterlite_server::shadow_result_aggregation::legs::LegRow;
use betterlite_server::shadow_result_aggregation::responses::LegAttemptCount;
use chrono::{DateTime, Utc};

use super::{ProbeStore, ProbeStoreError};

#[async_trait]
impl ShadowLifecycleStore for ProbeStore {
    type Error = ProbeStoreError;

    async fn load_targets(
        &self,
        keys: &[String],
        teams: &[String],
        users: &[String],
    ) -> Result<TargetRows, Self::Error> {
        Ok(TargetRows {
            keys: self
                .targets
                .keys
                .iter()
                .filter(|row| keys.contains(&row.token))
                .cloned()
                .collect(),
            teams: self
                .targets
                .teams
                .iter()
                .filter(|row| teams.contains(&row.team_id))
                .cloned()
                .collect(),
            users: self
                .targets
                .users
                .iter()
                .filter(|row| users.contains(&row.user_id))
                .cloned()
                .collect(),
        })
    }

    async fn sweep_finished(
        &self,
        ids: &[String],
        target_type: ShadowEvalTargetType,
    ) -> Result<(), Self::Error> {
        let mut legs = self.legs.lock().unwrap_or_else(PoisonError::into_inner);
        for leg in &mut *legs {
            if leg.target_type != target_type
                || !ids.contains(&leg.target_id)
                || leg.stopped_at.is_some()
            {
                continue;
            }
            let finished = leg.ends_at <= self.now
                || self.attempt_count(leg) >= leg.max_turns
                || !self.budget_open(leg);
            if finished {
                leg.stopped_at = Some(self.now);
            }
        }
        Ok(())
    }

    async fn claimed_targets(
        &self,
        targets: &[(ShadowEvalTargetType, Vec<String>)],
        direction: ShadowEvalDirection,
    ) -> Result<Vec<ClaimedTarget>, Self::Error> {
        let legs = self.legs.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(legs
            .iter()
            .filter(|leg| {
                leg.stopped_at.is_none()
                    && leg.direction == direction
                    && targets.iter().any(|(target_type, ids)| {
                        *target_type == leg.target_type && ids.contains(&leg.target_id)
                    })
            })
            .map(|leg| ClaimedTarget {
                group_id: leg.group_id.clone(),
                target_type: leg.target_type,
                target_id: leg.target_id.clone(),
            })
            .collect())
    }

    async fn create_legs(&self, legs: &[NewLeg]) -> Result<(), InsertRejection<Self::Error>> {
        if self.create_conflict {
            return Err(InsertRejection::UniqueViolation);
        }
        let rows: Vec<LegRow> = legs.iter().map(NewLeg::row).collect();
        self.legs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(rows);
        Ok(())
    }

    async fn seed_funnel(&self, job_ids: &[String]) -> Result<(), Self::Error> {
        if self.seeds_fail {
            return Err(ProbeStoreError {
                message: format!("the funnel seed for {} legs failed", job_ids.len()),
            });
        }
        Ok(())
    }

    async fn legs(&self, group_id: &str) -> Result<Vec<LegRow>, Self::Error> {
        Ok(self.legs_of(group_id))
    }

    async fn attempt_stats(&self, job_ids: &[String]) -> Result<Vec<LegAttemptCount>, Self::Error> {
        let legs = self.legs.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(legs
            .iter()
            .filter(|leg| job_ids.contains(&leg.id))
            .filter_map(|leg| {
                let count = self.attempt_count(leg);
                if count == 0 {
                    return None;
                }
                Some(LegAttemptCount {
                    job_id: leg.id.clone(),
                    attempt_count: count,
                    spend: self.attempt_spend(leg),
                })
            })
            .collect())
    }

    async fn stop_job(
        &self,
        job_id: &str,
        operator: &str,
        stamp: DateTime<Utc>,
    ) -> Result<u64, Self::Error> {
        let mut legs = self.legs.lock().unwrap_or_else(PoisonError::into_inner);
        let group: Vec<usize> = legs
            .iter()
            .enumerate()
            .filter(|(_, leg)| leg.group_id == job_id)
            .map(|(index, _)| index)
            .collect();
        let Some(first) = group.first().copied() else {
            return Ok(0);
        };
        let first_leg = legs.get(first).ok_or(ProbeStoreError {
            message: format!("the stop read lost leg {first}"),
        })?;
        let window_open = first_leg.ends_at > self.now;
        let still_sampling = group.iter().any(|index| {
            legs.get(*index).is_some_and(|leg| {
                leg.stopped_at.is_none()
                    && self.attempt_count(leg) < leg.max_turns
                    && self.budget_open(leg)
            })
        });
        if !window_open || !still_sampling {
            return Ok(0);
        }
        for index in group {
            let Some(leg) = legs.get_mut(index) else {
                continue;
            };
            if leg.stopped_by.is_none() && leg.ends_at > self.now {
                leg.stopped_by = Some(operator.to_owned());
                if leg.stopped_at.is_none() {
                    leg.stopped_at = Some(stamp);
                }
            }
        }
        Ok(1)
    }
}
