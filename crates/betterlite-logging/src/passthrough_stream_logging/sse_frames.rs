const SSE_FRAME_DELIMITERS: [&[u8]; 3] = [b"\r\n\r\n", b"\n\n", b"\r\r"];

// B005 owns the canonical split; this port keeps its three delimiters and last-boundary rule until B005 lands.
#[must_use]
pub fn split_complete_sse_frames(pending: &[u8]) -> (&[u8], &[u8]) {
    let mut boundary_end = 0;
    for delimiter in SSE_FRAME_DELIMITERS {
        let Some(index) = pending
            .windows(delimiter.len())
            .rposition(|window| window == delimiter)
        else {
            continue;
        };
        boundary_end = boundary_end.max(index.saturating_add(delimiter.len()));
    }
    if boundary_end == 0 {
        return (&[], pending);
    }
    pending.split_at(boundary_end)
}

// Python's bytes.strip removes VT as well, which u8::is_ascii_whitespace leaves in place.
#[must_use]
pub fn tail_is_blank(tail: &[u8]) -> bool {
    tail.iter()
        .all(|byte| byte.is_ascii_whitespace() || *byte == b'\x0b')
}
