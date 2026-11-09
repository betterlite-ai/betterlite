pub(super) fn bulk_reply(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("${}\r\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
    out
}

pub(super) fn array_reply(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        out.extend(bulk_reply(value));
    }
    out
}

pub(super) fn error_reply(text: &str) -> Vec<u8> {
    let mut reply = b"-".to_vec();
    reply.extend_from_slice(text.as_bytes());
    reply.extend_from_slice(b"\r\n");
    reply
}
