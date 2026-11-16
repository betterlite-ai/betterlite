use std::num::NonZeroUsize;
use std::time::{Duration, SystemTime};

use secrecy::SecretString;

pub const OBO_CACHE_MAX_ENTRIES: usize = 1000;
pub const OBO_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(OBO_CACHE_MAX_ENTRIES).unwrap();
pub const TOKEN_EXPIRY_MARGIN: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct CachedOboToken {
    pub access_token: SecretString,
    pub expires_at: SystemTime,
}
