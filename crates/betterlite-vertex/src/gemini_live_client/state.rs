use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

pub const MAX_TOOL_CALL_NAMES: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolCallNames {
    // the least recently used call id comes first, and insert caps the map at MAX_TOOL_CALL_NAMES
    pub entries: IndexMap<String, String>,
}

impl ToolCallNames {
    pub fn insert(&mut self, call_id: &str, name: &str) {
        if call_id.is_empty() || name.is_empty() {
            return;
        }
        if self.entries.shift_remove(call_id).is_none() {
            while self.entries.len() >= MAX_TOOL_CALL_NAMES {
                if self.entries.shift_remove_index(0).is_none() {
                    break;
                }
            }
        }
        self.entries.insert(call_id.to_owned(), name.to_owned());
    }

    pub fn refresh(&mut self, call_id: &str) {
        let Some(name) = self.entries.shift_remove(call_id) else {
            return;
        };
        self.entries.insert(call_id.to_owned(), name);
    }
}

#[derive(Debug, Default)]
pub struct LiveSessionState {
    pub tool_call_names: ToolCallNames,
    // Gemini Live can send usageMetadata in a standalone frame, buffered until the next response.done
    pub pending_usage_metadata: Option<Object>,
    pub unbilled_input_audio_bytes: u64,
}
