use sha2::{Digest as _, Sha256};

const DIGEST_LENGTH: usize = 8;
const IDENTIFIER_MAX_BYTES: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLogIndex<'a> {
    pub table: &'a str,
    pub name: &'a str,
    pub definition: &'a str,
}

impl<'a> RequestLogIndex<'a> {
    pub fn columns(self) -> Vec<&'a str> {
        let mut columns = Vec::new();
        let mut rest = self.definition;
        while let Some(start) = rest.find('"') {
            let after_start = &rest[start + 1..];
            let Some(end) = after_start.find('"') else {
                break;
            };
            let column = &after_start[..end];
            if !column.is_empty() {
                columns.push(column);
            }
            rest = &after_start[end + 1..];
        }
        columns
    }

    pub fn partition_index_name(self, partition: &str) -> String {
        let prefix = format!("{}_", self.table);
        let index_suffix = self.name.strip_prefix(&prefix).unwrap_or(self.name);
        let name = format!("{partition}_{index_suffix}");
        if name.len() <= IDENTIFIER_MAX_BYTES {
            return name;
        }
        let digest = Sha256::digest(name.as_bytes());
        let digest = hex::encode(digest);
        let digest = &digest[..DIGEST_LENGTH];
        let prefix_budget = IDENTIFIER_MAX_BYTES - DIGEST_LENGTH - 1;
        let kept = name
            .char_indices()
            .take_while(|(index, _)| *index <= prefix_budget)
            .map(|(index, character)| (index + character.len_utf8(), character))
            .take_while(|(end, _)| *end <= prefix_budget)
            .last()
            .map_or("", |(end, _)| &name[..end]);
        format!("{kept}_{digest}")
    }
}

pub const REQUEST_LOG_INDEXES: [RequestLogIndex<'static>; 2] = [
    RequestLogIndex {
        table: "LiteLLM_SpendLogs",
        name: "LiteLLM_SpendLogs_api_key_startTime_idx",
        definition: "(\"api_key\", \"startTime\")",
    },
    RequestLogIndex {
        table: "LiteLLM_SpendLogs",
        name: "LiteLLM_SpendLogs_litellm_call_id_idx",
        definition: "(\"litellm_call_id\")",
    },
];
