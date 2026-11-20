use betterlite_openai::reasoning_effort_capability::GroupReasoningEfforts;

use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::{ClientsideAuth, ModelGroupInfo};
use crate::model_group_info::row::ConfiguredLimits;

// The per-call running answer: the group, the totals, the reasoning answer and the last auth params.
pub(crate) struct AggregateState<'a> {
    pub(crate) model_group: &'a str,
    pub(crate) user_facing: &'a str,
    pub(crate) group: Option<ModelGroupInfo>,
    pub(crate) totals: Totals,
    pub(crate) reasoning: GroupReasoningEfforts,
    pub(crate) auth: Option<Option<Vec<ClientsideAuth>>>,
}

impl<'a> AggregateState<'a> {
    pub(crate) fn new(model_group: &'a str, user_facing: &'a str) -> Self {
        Self {
            model_group,
            user_facing,
            group: None,
            totals: Totals::default(),
            reasoning: GroupReasoningEfforts::default(),
            auth: None,
        }
    }

    // The totals, the last auth and the reasoning answer are applied once, after every deployment.
    pub(crate) fn finish(self) -> Option<ModelGroupInfo> {
        let mut group = self.group?;
        if let Some(tpm) = self.totals.tpm {
            group.tpm = Some(tpm);
        }
        if let Some(rpm) = self.totals.rpm {
            group.rpm = Some(rpm);
        }
        if let Some(itpm) = self.totals.itpm {
            group.itpm = Some(itpm);
        }
        if let Some(otpm) = self.totals.otpm {
            group.otpm = Some(otpm);
        }
        if let Some(auth) = self.auth.flatten() {
            group.configurable_clientside_auth_params = Some(auth);
        }
        group.supported_reasoning_efforts = self.reasoning.supported;
        Some(group)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Totals {
    tpm: Option<i64>,
    rpm: Option<i64>,
    itpm: Option<i64>,
    otpm: Option<i64>,
}

impl Totals {
    pub(crate) fn add(&mut self, limits: &ConfiguredLimits) -> Result<(), GroupInfoError> {
        self.tpm = add_limit(self.tpm, limits.tpm)?;
        self.rpm = add_limit(self.rpm, limits.rpm)?;
        self.itpm = add_limit(self.itpm, limits.itpm)?;
        self.otpm = add_limit(self.otpm, limits.otpm)?;
        Ok(())
    }
}

fn add_limit(total: Option<i64>, value: Option<i64>) -> Result<Option<i64>, GroupInfoError> {
    let Some(found) = value else {
        return Ok(total);
    };
    let Some(current) = total else {
        return Ok(Some(found));
    };
    current
        .checked_add(found)
        .map(Some)
        .ok_or(GroupInfoError::Totals)
}
