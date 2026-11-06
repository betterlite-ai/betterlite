use crate::engine::tiktoken::Encoding;

// The filenames are the sha1 of the tiktoken URL, the names LiteLLM ships the files under; the parts keep each included file under clippy's 1 MB include limit
const P50K_BASE: &[u8] = include_bytes!("../../fixtures/ec7223a39ce59f226a68acc30dc1af2788490e15");
const CL100K_PARTS: &[&[u8]] = &[
    include_bytes!("../../fixtures/parts/cl100k-0"),
    include_bytes!("../../fixtures/parts/cl100k-1"),
];
const O200K_PARTS: &[&[u8]] = &[
    include_bytes!("../../fixtures/parts/o200k-0"),
    include_bytes!("../../fixtures/parts/o200k-1"),
    include_bytes!("../../fixtures/parts/o200k-2"),
    include_bytes!("../../fixtures/parts/o200k-3"),
    include_bytes!("../../fixtures/parts/o200k-4"),
];
pub const ANTHROPIC_PARTS: &[&[u8]] = &[
    include_bytes!("../../fixtures/parts/anthropic-0"),
    include_bytes!("../../fixtures/parts/anthropic-1"),
];

#[must_use]
pub fn bundled(encoding: Encoding) -> Option<Vec<u8>> {
    match encoding {
        Encoding::Cl100kBase => Some(joined(CL100K_PARTS)),
        Encoding::O200kBase | Encoding::O200kHarmony => Some(joined(O200K_PARTS)),
        Encoding::P50kBase | Encoding::P50kEdit => Some(P50K_BASE.to_vec()),
        Encoding::R50kBase | Encoding::Gpt2 => None,
    }
}

#[must_use]
pub fn joined(parts: &[&[u8]]) -> Vec<u8> {
    let size = parts.iter().map(|part| part.len()).sum();
    let mut bytes = Vec::with_capacity(size);
    for part in parts {
        bytes.extend_from_slice(part);
    }
    bytes
}
