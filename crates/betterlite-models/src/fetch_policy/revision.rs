use sha1::{Digest as _, Sha1};

#[must_use]
pub fn git_blob_id(body: &[u8]) -> String {
    let header = format!("blob {}\0", body.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(body);
    hex::encode(hasher.finalize())
}
