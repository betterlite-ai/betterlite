use super::linked_rows::{LinkedKeys, ModelAccessGroupRow};

impl LinkedKeys for ModelAccessGroupRow {
    fn counter_key(&self) -> String {
        format!("spend:model_access_group:{}", self.access_group_name)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![format!("model_access_group:{}", self.access_group_name)]
    }
}

mod tests;
