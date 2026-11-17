// LiteLLM's DEFAULT_CHUNK_SIZE and DEFAULT_CHUNK_OVERLAP, read once at import and kept zero when set to zero
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexRagChunkDefaults {
    pub chunk_size: i64,
    pub chunk_overlap: i64,
}
