use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtlpError {
    pub message: String,
}

pub fn encode_otlp_response(content_type: Option<&str>, error: Option<&str>) -> (Vec<u8>, String) {
    let media_type = content_type
        .unwrap_or("application/x-protobuf")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if media_type == "application/json" {
        return match error {
            Some(message) => (
                serde_json::to_vec(&OtlpError {
                    message: message.to_owned(),
                })
                .unwrap_or_default(),
                "application/json".to_owned(),
            ),
            None => (b"{}".to_vec(), "application/json".to_owned()),
        };
    }
    match error {
        Some(message) => (encode_error(message), "application/x-protobuf".to_owned()),
        None => (Vec::new(), "application/x-protobuf".to_owned()),
    }
}

fn encode_error(message: &str) -> Vec<u8> {
    let bytes = message.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() + 2);
    output.push(0x0a);
    encode_varint(bytes.len(), &mut output);
    output.extend_from_slice(bytes);
    output
}

fn encode_varint(mut value: usize, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push(u8::try_from(value & 127).unwrap_or(127) | 128);
        value >>= 7;
    }
    output.push(u8::try_from(value).unwrap_or(0));
}
