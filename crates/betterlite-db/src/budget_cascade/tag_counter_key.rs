use super::linked_rows::{LinkedKeys, TagRow};

impl LinkedKeys for TagRow {
    fn counter_key(&self) -> String {
        format!("spend:tag:{}", self.tag_name)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![format!("tag:{}", self.tag_name)]
    }
}

mod tests;
