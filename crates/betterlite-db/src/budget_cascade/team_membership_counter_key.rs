use super::linked_rows::{LinkedKeys, TeamMembershipRow};

impl LinkedKeys for TeamMembershipRow {
    fn counter_key(&self) -> String {
        format!("spend:team_member:{}:{}", self.user_id, self.team_id)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![format!("{}_{}", self.team_id, self.user_id)]
    }
}

mod tests;
