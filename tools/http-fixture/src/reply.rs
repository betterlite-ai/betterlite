#[must_use]
pub fn reply(status: &str, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
    let mut answer = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        answer.push_str(name);
        answer.push_str(": ");
        answer.push_str(value);
        answer.push_str("\r\n");
    }
    answer.push_str("\r\n");
    answer.push_str(body);
    answer.into_bytes()
}
