use super::linked_rows::{LinkedKeys, OrganizationRow};

impl LinkedKeys for OrganizationRow {
    fn counter_key(&self) -> String {
        format!("spend:org:{}", self.organization_id)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![
            format!("org_id:{}", self.organization_id),
            format!("org_id:{}:with_budget", self.organization_id),
        ]
    }
}

mod tests;
