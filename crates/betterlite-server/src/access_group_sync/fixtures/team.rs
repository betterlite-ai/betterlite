use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use crate::access_group_sync::team::{
    AffectedGroupRow, TeamGroupRow, TeamSyncError, TeamSyncTransaction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamStep {
    Lock,
    Read,
    Affected,
    Attach,
    Detach,
}

#[derive(Clone, Copy)]
enum TeamWrite {
    Attach,
    Detach,
}

pub(crate) struct ScriptedTeamTransaction {
    pub(crate) calls: Mutex<Vec<TeamStep>>,
    pub(crate) missing_team: bool,
    pub(crate) team_group_ids: Option<Vec<String>>,
    pub(crate) groups: Mutex<BTreeMap<String, Option<Vec<String>>>>,
}

impl ScriptedTeamTransaction {
    pub(crate) const fn new(
        team_group_ids: Option<Vec<String>>,
        groups: BTreeMap<String, Option<Vec<String>>>,
    ) -> Self {
        let calls = Mutex::new(Vec::new());
        let groups = Mutex::new(groups);
        Self {
            calls,
            missing_team: false,
            team_group_ids,
            groups,
        }
    }

    fn read_rows(&self) -> Option<TeamGroupRow> {
        self.calls.lock().expect("calls").push(TeamStep::Read);
        self.team_group_ids
            .clone()
            .filter(|_| !self.missing_team)
            .map(|access_group_ids| TeamGroupRow { access_group_ids })
    }

    fn affected_rows(&self, team_id: &str, desired: &[String]) -> Vec<AffectedGroupRow> {
        self.calls.lock().expect("calls").push(TeamStep::Affected);
        let groups = self.groups.lock().expect("groups");
        let mut rows = Vec::new();
        for (access_group_id, assigned) in groups.iter() {
            let wanted = desired.contains(access_group_id);
            let member = assigned
                .as_ref()
                .is_some_and(|teams| teams.iter().any(|team| team == team_id));
            if wanted || member {
                rows.push(AffectedGroupRow {
                    access_group_id: access_group_id.clone(),
                });
            }
        }
        rows
    }

    fn write(
        &self,
        write: TeamWrite,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(match write {
            TeamWrite::Attach => TeamStep::Attach,
            TeamWrite::Detach => TeamStep::Detach,
        });
        let mut groups = self.groups.lock().expect("groups");
        match write {
            TeamWrite::Attach => {
                for (access_group_id, assigned) in groups.iter_mut() {
                    if !desired.contains(access_group_id) {
                        continue;
                    }
                    let teams = assigned.get_or_insert_default();
                    if !teams.iter().any(|team| team == team_id) {
                        teams.push(team_id.to_owned());
                    }
                }
            }
            TeamWrite::Detach => {
                for (access_group_id, assigned) in groups.iter_mut() {
                    if desired.contains(access_group_id) {
                        continue;
                    }
                    if let Some(teams) = assigned {
                        teams.retain(|team| team != team_id);
                    }
                }
            }
        }
        ready(Ok(()))
    }
}

impl TeamSyncTransaction for ScriptedTeamTransaction {
    fn lock_team(&self, _team_id: &str) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(TeamStep::Lock);
        async move { Ok(()) }
    }

    fn team_groups(
        &self,
        _team_id: &str,
    ) -> impl Future<Output = Result<Option<TeamGroupRow>, TeamSyncError>> + Send {
        async move { Ok(self.read_rows()) }
    }

    fn affected_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<Vec<AffectedGroupRow>, TeamSyncError>> + Send {
        async move { Ok(self.affected_rows(team_id, desired)) }
    }

    fn attach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.write(TeamWrite::Attach, team_id, desired)
    }

    fn detach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.write(TeamWrite::Detach, team_id, desired)
    }
}
