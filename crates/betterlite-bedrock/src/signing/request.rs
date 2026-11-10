use std::fmt::{self, Debug, Formatter};
pub struct AwsSignedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Debug for AwsSignedRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let headers: Vec<String> = self
            .headers
            .iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization")
                    || name.eq_ignore_ascii_case("x-amz-security-token")
                {
                    format!("{name}: <redacted>")
                } else {
                    format!("{name}: {value}")
                }
            })
            .collect();
        f.debug_struct("AwsSignedRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_length", &self.body.len())
            .finish()
    }
}
