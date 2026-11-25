// The core helper and public B901 runner share a schema while retaining their visibility boundaries.
macro_rules! define_input {
    ($visibility:vis) => {
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;

$visibility mod rows {
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
$visibility enum ChunkKind {
    None,
    Json,
    Object,
    Broken,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
$visibility enum QuotaKind {
    RateLimit,
    Internal,
}

}
use rows::{ChunkKind, QuotaKind};

#[derive(Clone, Deserialize, Serialize)]
$visibility struct Setup {
    $visibility model: String,
    $visibility litellm_metadata: Option<IndexMap<String, Json>>,
    $visibility custom_llm_provider: Option<String>,
    $visibility api_key: Option<String>,
    $visibility api_base: Option<String>,
    $visibility timeout: Option<f64>,
    $visibility first_message: Option<String>,
    $visibility kwargs: IndexMap<String, Json>,
}

#[derive(Clone, Deserialize, Serialize)]
$visibility struct Input {
    #[serde(default)]
    $visibility normalize_warmup: bool,
    #[serde(default = "default_deadline_millis")]
    $visibility deadline_millis: u64,
    $visibility setup: Setup,
    $visibility frames: Vec<String>,
    $visibility disconnect: String,
    $visibility scripts: Vec<ScriptRow>,
    $visibility quota: Option<QuotaRow>,
    $visibility sink: SinkRow,
}

#[derive(Clone, Deserialize, Serialize)]
$visibility struct ChunkRow {
    $visibility kind: ChunkKind,
    $visibility value: Option<Json>,
    $visibility event_type: Option<String>,
    $visibility dump: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
$visibility struct ScriptRow {
    $visibility fail: Option<String>,
    $visibility chunks: Vec<ChunkRow>,
    $visibility raise_after: Option<usize>,
    $visibility raise_message: String,
}

#[derive(Clone, Deserialize, Serialize)]
$visibility struct QuotaRow {
    $visibility kind: QuotaKind,
    $visibility message: String,
}

#[derive(Clone, Deserialize, Serialize)]
$visibility struct SinkRow {
    $visibility fails_at: Option<usize>,
    $visibility message: String,
}

impl fmt::Debug for Input {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedInput")
            .finish_non_exhaustive()
    }
}

const fn default_deadline_millis() -> u64 {
    120_000
}

#[derive(Debug, thiserror::Error)]
$visibility enum InputError {
    #[error("managed transcript exceeds its frame, capture or execution limit")]
    Limit,
    #[error("managed transcript cannot be encoded")]
    Encode(#[from] serde_json::Error),
}

impl Input {
    $visibility fn check_bounds(
        &self,
        max_frames: usize,
        max_frame_bytes: usize,
        remaining: usize,
    ) -> Result<(usize, usize), InputError> {
        let count = self.scripts.iter().fold(
            self.frames
                .len()
                .saturating_add(usize::from(self.setup.first_message.is_some())),
            |total, script| total.saturating_add(script.chunks.len()).saturating_add(1),
        );
        if count > max_frames || self.deadline_millis == 0 || self.deadline_millis > 120_000 {
            return Err(InputError::Limit);
        }
        let size = serde_json::to_vec(self)?.len();
        if size > remaining || serde_json::to_vec(&self.setup)?.len() > max_frame_bytes {
            return Err(InputError::Limit);
        }
        for frame in self.frames.iter().chain(self.setup.first_message.iter()) {
            if frame.len() > max_frame_bytes {
                return Err(InputError::Limit);
            }
        }
        for chunk in self.scripts.iter().flat_map(|script| &script.chunks) {
            let chunk_size = match &chunk.dump {
                Some(dump) => dump.len(),
                None => serde_json::to_vec(&chunk.value)?.len(),
            };
            if chunk_size > max_frame_bytes {
                return Err(InputError::Limit);
            }
        }
        Ok((count, size))
    }
}



    };
}

pub(crate) use define_input;
