use super::decode::utf8_code_points;

// A Python str as generalized UTF-8, so the lone surrogates json.loads accepts keep their own code point
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordText(Vec<u8>);

impl RecordText {
    #[must_use]
    pub fn from_code_points(code_points: &[u32]) -> Self {
        let mut bytes = Vec::with_capacity(code_points.len());
        for &code_point in code_points {
            push_code_point(&mut bytes, code_point);
        }
        Self(bytes)
    }

    // str.encode("utf-8", "replace").decode("utf-8") with a chosen stand-in for each lone surrogate
    #[must_use]
    pub fn replaced(&self, stand_in: char) -> String {
        let code_points = utf8_code_points(&self.0).unwrap_or_default();
        code_points
            .into_iter()
            .map(|code_point| char::from_u32(code_point).unwrap_or(stand_in))
            .collect()
    }
}

impl From<&str> for RecordText {
    fn from(text: &str) -> Self {
        Self(text.as_bytes().to_vec())
    }
}

impl AsRef<[u8]> for RecordText {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

fn push_code_point(bytes: &mut Vec<u8>, code_point: u32) {
    if let Some(character) = char::from_u32(code_point) {
        let mut buffer = [0_u8; 4];
        bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
    } else {
        // a surrogate takes the three-byte form UTF-8 would give it if it were a scalar value
        let continuation = |shift: u32| {
            let low_bits = u8::try_from(code_point.wrapping_shr(shift) & 0x3F).unwrap_or(0);
            0x80_u8.wrapping_add(low_bits)
        };
        bytes.extend([0xED, continuation(6), continuation(0)]);
    }
}
