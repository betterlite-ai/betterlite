use sha2::{Digest as _, Sha256};

const DIGEST_LENGTH: usize = 8;
const IDENTIFIER_MAX_BYTES: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLogIndex<'definition> {
    pub table: &'definition str,
    pub name: &'definition str,
    pub definition: &'definition str,
}

impl<'definition> RequestLogIndex<'definition> {
    #[must_use]
    pub fn columns(self) -> Vec<&'definition str> {
        let mut columns = Vec::new();
        let mut open = None;
        for (index, character) in self.definition.char_indices() {
            if character != '"' {
                continue;
            }
            if let Some(start) = open.take()
                && start < index
                && let Some(column) = self.definition.get(start..index)
            {
                columns.push(column);
            } else {
                open = index.checked_add(1);
            }
        }
        columns
    }

    #[must_use]
    pub fn partition_index_name(self, partition: &str) -> String {
        let prefix = format!("{}_", self.table);
        let index_suffix = self.name.strip_prefix(&prefix).unwrap_or(self.name);
        let name = format!("{partition}_{index_suffix}");
        if name.len() <= IDENTIFIER_MAX_BYTES {
            return name;
        }
        let digest_bytes = Sha256::digest(name.as_bytes());
        let digest_hex = hex::encode(digest_bytes);
        let digest = digest_hex.chars().take(DIGEST_LENGTH).collect::<String>();
        let prefix_budget = IDENTIFIER_MAX_BYTES - DIGEST_LENGTH - 1;
        let kept = name
            .char_indices()
            .take_while(|(index, _)| *index <= prefix_budget)
            .filter_map(|(index, character)| {
                index
                    .checked_add(character.len_utf8())
                    .map(|end| (end, character))
            })
            .take_while(|(end, _)| *end <= prefix_budget)
            .last()
            .map_or("", |(end, _)| name.get(..end).unwrap_or(""));
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
