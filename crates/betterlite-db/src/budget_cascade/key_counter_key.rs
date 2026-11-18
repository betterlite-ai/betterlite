use super::linked_rows::{KeyRow, LinkedKeys};

impl LinkedKeys for KeyRow {
    fn counter_key(&self) -> String {
        format!("spend:key:{}", self.token)
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![self.token.clone()]
    }
}

mod tests;
