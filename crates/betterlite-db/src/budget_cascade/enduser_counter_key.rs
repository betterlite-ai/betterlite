use super::linked_rows::{EndUserRow, LinkedKeys};

impl LinkedKeys for EndUserRow {
    fn counter_key(&self) -> String {
        format!("spend:end_user:{}", self.user_id)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![format!("end_user_id:{}", self.user_id)]
    }
}

mod tests;
