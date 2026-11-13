use std::fs::{self, File};
use std::io::{ErrorKind, Read as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles};
use rust_decimal::Decimal;
use rustix::fs::{Mode, OFlags, open};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use tokio::runtime::Runtime;

use super::credentials::Credentials;
use super::fetch::SessionFetch;
use super::fetch::error::FetchError;
use super::session::{Session, decimal};

const TTL_SECONDS: u32 = 5;
const GROUP_AND_OTHER_BITS: u32 = 0o077;
const MILLISECOND_SCALE: u32 = 3;

pub trait Clock {
    fn now_millis(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }
}

// The same JSON LiteLLM's script keeps, so both can share one directory
#[derive(Serialize)]
struct Stored<'a> {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    fetched_at: Decimal,
    session: Option<&'a Session>,
}

#[derive(Deserialize)]
struct Entry<'a> {
    #[serde(borrow)]
    fetched_at: &'a RawValue,
    #[serde(borrow)]
    session: Option<&'a RawValue>,
}

enum Lookup {
    Fresh(Option<Session>),
    Stale,
}

pub struct SessionCache<'c> {
    pub directory: &'c Path,
    pub owner_uid: u32,
    pub files: &'c PrivateFiles,
    pub runtime: &'c Runtime,
    pub clock: &'c dyn Clock,
}

#[must_use]
pub fn cache_path(directory: &Path, credentials: &Credentials, session_id: &str) -> PathBuf {
    let mut identity = Sha256::new();
    identity.update(credentials.base_url.as_bytes());
    identity.update(b"\n");
    identity.update(credentials.api_key.expose_secret().as_bytes());
    identity.update(b"\n");
    identity.update(session_id.as_bytes());
    directory.join(hex::encode(identity.finalize()))
}

impl SessionCache<'_> {
    // A shared temp root lets another local user pre-create the directory, so it must be ours and private first
    fn directory_is_private(&self) -> bool {
        let missing = fs::symlink_metadata(self.directory)
            .is_err_and(|source| source.kind() == ErrorKind::NotFound);
        if missing
            && self
                .runtime
                .block_on(self.files.ensure_private_dir(self.directory))
                .is_err()
        {
            return false;
        }
        // lstat, so a link to a directory is not a directory here
        fs::symlink_metadata(self.directory).is_ok_and(|found| {
            found.is_dir()
                && found.uid() == self.owner_uid
                && found.mode() & GROUP_AND_OTHER_BITS == 0
        })
    }

    // O_NOFOLLOW refuses an entry that was swapped for a link
    fn lookup(&self, path: &Path) -> Lookup {
        if !self.directory_is_private() {
            return Lookup::Stale;
        }
        let Ok(descriptor) = open(path, OFlags::RDONLY.union(OFlags::NOFOLLOW), Mode::empty())
        else {
            return Lookup::Stale;
        };
        let mut raw = Vec::new();
        if File::from(descriptor).read_to_end(&mut raw).is_err() {
            return Lookup::Stale;
        }
        let Ok(entry) = serde_json::from_slice::<Entry<'_>>(&raw) else {
            return Lookup::Stale;
        };
        let Some(fetched_at) = decimal(entry.fetched_at) else {
            return Lookup::Stale;
        };
        let now = Decimal::new(self.clock.now_millis(), MILLISECOND_SCALE);
        match now.checked_sub(fetched_at) {
            Some(age) if age < Decimal::from(TTL_SECONDS) => Lookup::Fresh(
                entry
                    .session
                    .and_then(|raw_session| Session::from_json(raw_session.get().as_bytes())),
            ),
            Some(_) | None => Lookup::Stale,
        }
    }

    // Staged beside the entry and renamed into place, so a refresh reading it never sees a torn write
    fn store(&self, path: &Path, session: Option<&Session>) {
        let stored = Stored {
            fetched_at: Decimal::new(self.clock.now_millis(), MILLISECOND_SCALE),
            session,
        };
        let Ok(bytes) = serde_json::to_vec(&stored) else {
            return;
        };
        if !self.directory_is_private() {
            return;
        }
        let document = PrivateDocument {
            suffix: None,
            bytes,
        };
        // a cache that cannot be written only costs the next refresh a lookup
        let _written = self
            .runtime
            .block_on(self.files.write_private(path, document));
    }
}

// An answer the gateway settled (a session, or a 4xx) is kept for the TTL; a failed lookup is retried at once
pub fn load_session(
    cache: &SessionCache<'_>,
    credentials: &Credentials,
    session_id: &str,
    fetch: &dyn SessionFetch,
) -> Result<Option<Session>, FetchError> {
    let path = cache_path(cache.directory, credentials, session_id);
    if let Lookup::Fresh(session) = cache.lookup(&path) {
        return Ok(session);
    }
    let fetched = fetch.fetch(credentials, session_id)?;
    if fetched.definitive {
        cache.store(&path, fetched.session.as_ref());
    }
    Ok(fetched.session)
}

mod tests;
