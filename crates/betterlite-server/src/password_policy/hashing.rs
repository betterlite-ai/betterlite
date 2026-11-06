use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use scrypt::{Params, scrypt};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::task::{JoinError, JoinSet};

pub const SCRYPT_PREFIX: &str = "scrypt:";
const SALT_LENGTH: usize = 16;
const KEY_LENGTH: usize = 32;
const LOG_N: u8 = 14;
const BLOCK_SIZE: u32 = 8;
const PARALLELISM: u32 = 1;
const SHA256_HEX_LENGTH: usize = 64;

#[derive(Debug, Error)]
pub enum HashError {
    #[error("the scrypt parameters are invalid")]
    Params,
    #[error("the scrypt output length is invalid")]
    OutputLength,
    #[error("could not draw a random salt")]
    Random(#[source] getrandom::Error),
    #[error("the hashing pool is closed")]
    PoolClosed,
    #[error("the hashing task failed")]
    Task(#[source] Box<JoinError>),
    #[error("the hashing task returned no result")]
    NoResult,
}

pub trait SaltSource: Send + Sync {
    fn fill(&self, salt: &mut [u8; SALT_LENGTH]) -> Result<(), HashError>;
}

pub struct OsSalts;

impl SaltSource for OsSalts {
    fn fill(&self, salt: &mut [u8; SALT_LENGTH]) -> Result<(), HashError> {
        getrandom::fill(salt).map_err(HashError::Random)
    }
}

// scrypt at N=16384, r=8 holds 16 MiB while it runs, so the pool bounds both CPU and memory.
pub struct PasswordHasher<S> {
    salts: S,
    workers: Arc<Semaphore>,
}

impl<S: SaltSource> PasswordHasher<S> {
    #[must_use]
    pub fn new(salts: S, max_concurrent: usize) -> Self {
        Self {
            salts,
            workers: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    pub async fn hash(&self, password: &SecretString) -> Result<String, HashError> {
        let mut salt = [0_u8; SALT_LENGTH];
        self.salts.fill(&mut salt)?;
        let key = self.derive(password, salt.to_vec()).await?;
        let mut stored = Vec::with_capacity(SALT_LENGTH + KEY_LENGTH);
        stored.extend_from_slice(&salt);
        stored.extend_from_slice(&key);
        Ok(format!("{SCRYPT_PREFIX}{}", STANDARD.encode(stored)))
    }

    // Any failure to check an scrypt hash is a mismatch, as in LiteLLM's blanket except.
    pub async fn verify(&self, password: &SecretString, stored: &str) -> bool {
        let Some(encoded) = stored.strip_prefix(SCRYPT_PREFIX) else {
            return verify_sha256(password, stored);
        };
        // Python's b64decode encodes the text as ASCII first, so any other character fails the verify.
        if !encoded.is_ascii() {
            return false;
        }
        let kept: String = encoded
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || "+/=".contains(*character))
            .collect();
        let Ok(raw) = STANDARD.decode(kept) else {
            return false;
        };
        let (salt, expected) = raw.split_at(raw.len().min(SALT_LENGTH));
        match self.derive(password, salt.to_vec()).await {
            Ok(key) => bool::from(key.as_slice().ct_eq(expected)),
            Err(_failure) => false,
        }
    }

    async fn derive(
        &self,
        password: &SecretString,
        salt: Vec<u8>,
    ) -> Result<[u8; KEY_LENGTH], HashError> {
        let permit = Arc::clone(&self.workers)
            .acquire_owned()
            .await
            .map_err(|_closed| HashError::PoolClosed)?;
        let owned = password.clone();
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let params = Params::new(LOG_N, BLOCK_SIZE, PARALLELISM, KEY_LENGTH)
                .map_err(|_invalid| HashError::Params)?;
            let mut key = [0_u8; KEY_LENGTH];
            scrypt(owned.expose_secret().as_bytes(), &salt, &params, &mut key)
                .map_err(|_invalid| HashError::OutputLength)?;
            Ok(key)
        });
        let joined = tasks.join_next().await;
        drop(permit);
        match joined {
            Some(Ok(derived)) => derived,
            Some(Err(error)) => Err(HashError::Task(Box::new(error))),
            None => Err(HashError::NoResult),
        }
    }
}

#[must_use]
pub fn is_sha256_hex(stored: &str) -> bool {
    let lowercase_hex = stored
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    stored.len() == SHA256_HEX_LENGTH && lowercase_hex
}

// The legacy format stores sha256(password) as 64 lowercase hex characters.
fn verify_sha256(password: &SecretString, stored: &str) -> bool {
    if !is_sha256_hex(stored) {
        return false;
    }
    let digest = hex::encode(Sha256::digest(password.expose_secret().as_bytes()));
    bool::from(digest.as_bytes().ct_eq(stored.as_bytes()))
}
