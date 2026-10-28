use super::AssemblyError;

// betterlite-secrets::secret_codec::python_base64::decode_lenient filters the same alphabet and
// checks the same trailing-pad rule, but it is pub(crate) and bounds only input length, not a
// running decoded-size limit; its crate also pulls in encryption dependencies unrelated to
// streaming. Out of this task's scope.paths, so the check is reimplemented here against the
// running `limit` instead of exporting or relocating that helper.
pub(super) fn normalize(fragment: &str, limit: usize) -> Result<Vec<u8>, AssemblyError> {
    if !fragment.is_ascii() {
        return Err(AssemblyError::NonAsciiAudio);
    }
    let mut symbols = Vec::new();
    let mut padding = 0_usize;
    for byte in fragment.bytes() {
        match byte {
            b'=' => padding = padding.saturating_add(1),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                padding = 0;
                symbols.push(byte);
                let decoded_size = (symbols.len() >> 2)
                    .saturating_mul(3)
                    .saturating_add((symbols.len() & 3).saturating_sub(1));
                if decoded_size > limit {
                    return Err(AssemblyError::AudioTooLarge { limit });
                }
            }
            _ => {}
        }
    }
    match symbols.len() & 3 {
        1 => Err(AssemblyError::InvalidAudioPadding),
        2 if padding < 2 => Err(AssemblyError::InvalidAudioPadding),
        3 if padding == 0 => Err(AssemblyError::InvalidAudioPadding),
        _ => Ok(symbols),
    }
}
