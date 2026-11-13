use std::fmt::{self, Debug, Formatter};
pub struct AwsSignedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Debug for AwsSignedRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("AwsSignedRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.len())
            .finish()
    }
}
