const OPENAI_HINT: &str = "Each line must be a JSON object with keys custom_id, method, url, body";
const VERTEX_HINT: &str = "A passthrough upload takes native Vertex batch rows, so each line must be a JSON object with a request key";
const KEY_BITS: [u8; 4] = [1, 2, 4, 8];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineShape {
    OpenAi,
    VertexPassthrough,
}
impl LineShape {
    pub const fn hint(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_HINT,
            Self::VertexPassthrough => VERTEX_HINT,
        }
    }
    pub(super) const fn required_keys(self) -> &'static [&'static str] {
        match self {
            Self::OpenAi => &["custom_id", "method", "url", "body"],
            Self::VertexPassthrough => &["request"],
        }
    }
    pub(super) fn key_bit(self, key: &str) -> Option<u8> {
        let index = self
            .required_keys()
            .iter()
            .position(|required| *required == key)?;
        KEY_BITS.get(index).copied()
    }
    pub(super) fn first_missing(self, seen: u8) -> Option<&'static str> {
        let keys = self.required_keys();
        for (index, key) in keys.iter().enumerate() {
            let Some(bit) = KEY_BITS.get(index) else {
                continue;
            };
            if seen.bitand(*bit) == 0 {
                return Some(*key);
            }
        }
        None
    }
}
