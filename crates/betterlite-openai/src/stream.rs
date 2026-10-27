use std::mem;

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = event_end(&self.buffer) {
            let event = self.buffer.drain(..end).collect::<Vec<_>>();
            let payload = event_payload(&event);
            if !payload.is_empty() {
                events.push(payload);
            }
        }
        events
    }

    pub fn finish(&mut self) -> Vec<Vec<u8>> {
        let remainder = mem::take(&mut self.buffer);
        let payload = event_payload(&remainder);
        if payload.is_empty() {
            Vec::new()
        } else {
            vec![payload]
        }
    }
}

fn event_end(buffer: &[u8]) -> Option<usize> {
    let line_end = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .and_then(|position| position.checked_add(2));
    if line_end.is_some() {
        return line_end;
    }
    let position = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    position.checked_add(4)
}

fn event_payload(event: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    for line in event.split(|byte| *byte == b'\n' || *byte == b'\r') {
        let Some(data_line) = line.strip_prefix(b"data:") else {
            continue;
        };
        let line_payload = data_line.strip_prefix(b" ").unwrap_or(data_line);
        if !payload.is_empty() {
            payload.push(b'\n');
        }
        payload.extend_from_slice(line_payload);
    }
    payload
}

mod tests;
