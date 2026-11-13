use crate::engine::tiktoken::Encoding;
// The filenames are the sha1 of the tiktoken URL, the names LiteLLM ships the files under; each file is included whole
const P50K_BASE: &[u8] = include_bytes!("../../fixtures/ec7223a39ce59f226a68acc30dc1af2788490e15");
#[expect(
    clippy::large_include_file,
    reason = "The pinned cl100k vocabulary must be embedded byte-for-byte"
)]
const CL100K: &[u8] = include_bytes!("../../fixtures/9b5ad71b2ce5302211f9c61530b329a4922fc6a4");
#[expect(
    clippy::large_include_file,
    reason = "The pinned o200k vocabulary must be embedded byte-for-byte"
)]
const O200K: &[u8] = include_bytes!("../../fixtures/fb374d419588a4632f3f557e76b4b70aebbca790");
#[expect(
    clippy::large_include_file,
    reason = "The pinned anthropic tokenizer must be embedded byte-for-byte"
)]
pub const ANTHROPIC: &[u8] = include_bytes!("../../fixtures/anthropic_tokenizer.json");
#[must_use]
pub fn bundled(encoding: Encoding) -> Option<Vec<u8>> {
    match encoding {
        Encoding::Cl100kBase => Some(CL100K.to_vec()),
        Encoding::O200kBase | Encoding::O200kHarmony => Some(O200K.to_vec()),
        Encoding::P50kBase | Encoding::P50kEdit => Some(P50K_BASE.to_vec()),
        Encoding::R50kBase | Encoding::Gpt2 => None,
    }
}
