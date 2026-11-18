use super::linked_rows::{LinkedKeys, ProjectRow};

impl LinkedKeys for ProjectRow {
    fn counter_key(&self) -> String {
        format!("spend:project:{}", self.project_id)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![format!("project_id:{}", self.project_id)]
    }
}

mod tests;
