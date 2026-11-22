use std::collections::BTreeMap;

use serde::Deserialize;

use super::targets::TargetKey;
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::job::ShadowJob;
use crate::shadow_eval_models::leg::ShadowLeg;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LabelRow {
    pub target_id: String,
    #[serde(default)]
    pub target_alias: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
}

// which label table answers a read; a key, a team and a user each store their label differently
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelKind {
    Key,
    Team,
    User,
}

// _target_labels: a key's (alias, masked name), a team's (alias, None), a user's (email, None)
#[must_use]
pub fn target_labels(
    key_rows: &[LabelRow],
    team_rows: &[LabelRow],
    user_rows: &[LabelRow],
) -> BTreeMap<TargetKey, (Option<String>, Option<String>)> {
    let entries = key_rows
        .iter()
        .map(|row| (ShadowEvalTargetType::Key, row))
        .chain(
            team_rows
                .iter()
                .map(|row| (ShadowEvalTargetType::Team, row)),
        )
        .chain(
            user_rows
                .iter()
                .map(|row| (ShadowEvalTargetType::User, row)),
        );
    let mut labels = BTreeMap::new();
    for (target_type, row) in entries {
        let key = TargetKey {
            target_type,
            target_id: row.target_id.clone(),
        };
        labels.insert(key, (row.target_alias.clone(), row.key_name.clone()));
    }
    labels
}

// _target_ids_of: the sorted unique ids of one kind that the responses scope
#[must_use]
pub fn target_ids_of(responses: &[ShadowJob], target_type: ShadowEvalTargetType) -> Vec<String> {
    let mut ids: Vec<String> = responses
        .iter()
        .flat_map(|job| job.targets.iter())
        .filter(|target| target.target_type == target_type)
        .map(|target| target.target_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

// _with_target_labels: one batched read per kind answers every target, and a deleted one reads None
#[must_use]
pub fn label_responses(
    responses: &[ShadowJob],
    key_rows: &[LabelRow],
    team_rows: &[LabelRow],
    user_rows: &[LabelRow],
) -> Vec<ShadowJob> {
    let labels = target_labels(key_rows, team_rows, user_rows);
    responses
        .iter()
        .map(|job| ShadowJob {
            targets: job
                .targets
                .iter()
                .map(|target| {
                    let label = labels.get(&TargetKey {
                        target_type: target.target_type,
                        target_id: target.target_id.clone(),
                    });
                    ShadowLeg {
                        target_alias: label.and_then(|pair| pair.0.clone()),
                        key_name: label.and_then(|pair| pair.1.clone()),
                        ..target.clone()
                    }
                })
                .collect(),
            ..job.clone()
        })
        .collect()
}
