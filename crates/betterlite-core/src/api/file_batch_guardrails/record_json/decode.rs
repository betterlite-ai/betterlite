pub const MAX_CODE_POINT: u32 = 0x0010_FFFF;

// json.loads(bytes) picks the codec with json.detect_encoding and decodes with surrogatepass
#[must_use]
pub fn line_code_points(line: &[u8]) -> Option<Vec<u32>> {
    match line {
        [0x00, 0x00, 0xFE, 0xFF, rest @ ..] => wide_code_points(rest, u32::from_be_bytes),
        [0xFF, 0xFE, 0x00, 0x00, rest @ ..] => wide_code_points(rest, u32::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16_code_points(rest, u16::from_be_bytes),
        [0xFF, 0xFE, rest @ ..] => utf16_code_points(rest, u16::from_le_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => utf8_code_points(rest),
        [0x00, 0x00, _, _, ..] => wide_code_points(line, u32::from_be_bytes),
        [0x00, _, _, _, ..] | [0x00, _] => utf16_code_points(line, u16::from_be_bytes),
        [_, 0x00, 0x00, 0x00, ..] => wide_code_points(line, u32::from_le_bytes),
        [_, 0x00, _, _, ..] | [_, 0x00] => utf16_code_points(line, u16::from_le_bytes),
        _ => utf8_code_points(line),
    }
}

// surrogatepass lets a surrogate through like any other three-byte form; overlong and out-of-range forms still fail
#[must_use]
pub fn utf8_code_points(bytes: &[u8]) -> Option<Vec<u32>> {
    let mut code_points = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&lead, tail)) = rest.split_first() {
        let (extra, payload, minimum) = match lead.leading_ones() {
            0 => (0_usize, lead, 0_u32),
            2 => (1, lead & 0x1F, 0x80),
            3 => (2, lead & 0x0F, 0x0800),
            4 => (3, lead & 0x07, 0x0001_0000),
            _ => return None,
        };
        let mut code_point = u32::from(payload);
        for byte in tail.get(..extra)? {
            if byte.leading_ones() != 1 {
                return None;
            }
            code_point = code_point.wrapping_shl(6) | u32::from(byte & 0x3F);
        }
        if !(minimum..=MAX_CODE_POINT).contains(&code_point) {
            return None;
        }
        code_points.push(code_point);
        rest = tail.get(extra..)?;
    }
    Some(code_points)
}

fn utf16_code_points(bytes: &[u8], read: fn([u8; 2]) -> u16) -> Option<Vec<u32>> {
    let pairs = bytes.chunks_exact(2);
    let whole = pairs.remainder().is_empty();
    let units = pairs.filter_map(|pair| <[u8; 2]>::try_from(pair).ok().map(read));
    let decoded = char::decode_utf16(units)
        .map(|unit| unit.map_or_else(|lone| u32::from(lone.unpaired_surrogate()), u32::from));
    whole.then(|| decoded.collect())
}

fn wide_code_points(bytes: &[u8], read: fn([u8; 4]) -> u32) -> Option<Vec<u32>> {
    let quads = bytes.chunks_exact(4);
    if !quads.remainder().is_empty() {
        return None;
    }
    quads
        .map(|quad| {
            let code_point = <[u8; 4]>::try_from(quad).ok().map(read);
            code_point.filter(|value| *value <= MAX_CODE_POINT)
        })
        .collect()
}
