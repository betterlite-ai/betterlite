use std::fs::{self, File};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use serde::Deserialize;
use serde::de::IgnoredAny;

use super::credentials::printable;

// 4 MiB
pub const SCAN_LIMIT_BYTES: u64 = 0x0040_0000;
const SYNTHETIC_MODEL: &str = "<synthetic>";
const ASSISTANT_TYPE: &str = "assistant";
const MODEL_CACHE_DIRECTORY: &str = "cache";
const MODEL_CACHE_FILE: &str = "gateway-models.json";
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "isSidechain")]
    sidechain: Option<bool>,
    #[serde(rename = "agentId")]
    agent_id: Option<String>,
    message: Option<Message>,
}
#[derive(Deserialize)]
struct Message {
    model: Option<String>,
}
#[derive(Deserialize)]
struct ModelCache {
    models: Vec<CacheEntry>,
}
// An entry that is not an object with a string id and display_name is skipped, the rest of the file still counts
#[derive(Deserialize)]
#[serde(untagged)]
enum CacheEntry {
    Listed { id: String, display_name: String },
    Unlisted(IgnoredAny),
}
// A missing or unreadable file, or no qualifying line in the last 4 MiB, is no model at all
pub(crate) fn latest_transcript_model(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let size = file.seek(SeekFrom::End(0)).ok()?;
    let _start = file
        .seek(SeekFrom::Start(size.saturating_sub(SCAN_LIMIT_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    let _read = file.take(SCAN_LIMIT_BYTES).read_to_end(&mut tail).ok()?;
    tail.split(|byte| *byte == b'\n').rev().find_map(line_model)
}

// A `<synthetic>` model is Claude Code's marker for a locally produced message, not a served model
fn line_model(line: &[u8]) -> Option<String> {
    let item = serde_json::from_slice::<Line>(line).ok()?;
    if item.kind.as_deref() != Some(ASSISTANT_TYPE)
        || item.sidechain == Some(true)
        || item.agent_id.is_some_and(|id| !id.is_empty())
    {
        return None;
    }
    let model = printable(&item.message?.model?);
    (!model.is_empty() && model != SYNTHETIC_MODEL).then_some(model)
}

pub(crate) fn model_label(model: &str, config_dir: &Path) -> String {
    let bare = model.rsplit_once('/').map_or(model, |(_, tail)| tail);
    let path = config_dir
        .join(MODEL_CACHE_DIRECTORY)
        .join(MODEL_CACHE_FILE);
    let Ok(raw) = fs::read(path) else {
        return bare.to_owned();
    };
    let Ok(cache) = serde_json::from_slice::<ModelCache>(&raw) else {
        return bare.to_owned();
    };
    cache
        .models
        .into_iter()
        .find_map(|entry| match entry {
            CacheEntry::Listed { id, display_name } if id == model || id == bare => {
                let label = printable(&display_name);
                (!label.is_empty()).then_some(label)
            }
            CacheEntry::Listed { .. } | CacheEntry::Unlisted(_) => None,
        })
        .unwrap_or_else(|| bare.to_owned())
}

mod tests;
