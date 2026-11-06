use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use thiserror::Error;
use tokio::fs;
use url::Url;
use uuid::Uuid;

use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::tiktoken::{Encoding, TiktokenEngine};

pub mod gpt2;

const FETCH_TIMEOUT: Duration = Duration::from_secs(600);

const R50K_URL: &str = "https://openaipublic.blob.core.windows.net/encodings/r50k_base.tiktoken";
const R50K_SHA256: &str = "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930";
const GPT2_VOCAB_URL: &str =
    "https://openaipublic.blob.core.windows.net/gpt-2/encodings/main/vocab.bpe";
const GPT2_VOCAB_SHA256: &str = "1ce1664773c50f3e0cc8842619a93edc4624525b728b188a9e0be33b7726adc5";
const GPT2_ENCODER_URL: &str =
    "https://openaipublic.blob.core.windows.net/gpt-2/encodings/main/encoder.json";
const GPT2_ENCODER_SHA256: &str =
    "196139668be63f3b5d6574427317ae82f612a97c5d1cdaf36ed2256dbf636783";

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("the tokenizer URL is invalid")]
    Url(#[source] url::ParseError),
    #[error("the tokenizer file could not be fetched")]
    Request(#[source] TransportError),
    #[error("fetching {url} returned status {status}")]
    Status { url: String, status: u16 },
    #[error("the downloaded file {url} does not match its sha256")]
    HashMismatch { url: String },
    #[error("the cached tokenizer at {path} could not be read")]
    CacheRead {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the tokenizer cache at {path} could not be written")]
    CacheWrite {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the gpt2 vocabulary could not be converted")]
    Gpt2(#[from] gpt2::Gpt2Error),
    #[error(transparent)]
    Engine(#[from] EngineError),
}

// tiktoken writes fetched files into TIKTOKEN_CACHE_DIR; betterlite keeps them in memory unless an operator sets a cache
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSource {
    pub r50k_url: String,
    pub r50k_sha256: String,
    pub gpt2_vocabulary_url: String,
    pub gpt2_vocabulary_sha256: String,
    pub gpt2_encoder_url: String,
    pub gpt2_encoder_sha256: String,
}

impl Default for RemoteSource {
    fn default() -> Self {
        Self {
            r50k_url: R50K_URL.to_owned(),
            r50k_sha256: R50K_SHA256.to_owned(),
            gpt2_vocabulary_url: GPT2_VOCAB_URL.to_owned(),
            gpt2_vocabulary_sha256: GPT2_VOCAB_SHA256.to_owned(),
            gpt2_encoder_url: GPT2_ENCODER_URL.to_owned(),
            gpt2_encoder_sha256: GPT2_ENCODER_SHA256.to_owned(),
        }
    }
}

pub struct TiktokenFetcher {
    client: Arc<ProxiedClient>,
    cache_dir: Option<PathBuf>,
    source: RemoteSource,
    memory: Mutex<BTreeMap<String, Arc<[u8]>>>,
}

impl TiktokenFetcher {
    #[must_use]
    pub const fn with_source(
        client: Arc<ProxiedClient>,
        cache_dir: Option<PathBuf>,
        source: RemoteSource,
    ) -> Self {
        Self {
            client,
            cache_dir,
            source,
            memory: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn engine(&self, encoding: Encoding) -> Result<Arc<TiktokenEngine>, FetchError> {
        match assets::bundled(encoding) {
            Some(data) => Ok(Arc::new(TiktokenEngine::from_bpe_file(encoding, &data)?)),
            None => self.fetched_engine(encoding).await,
        }
    }

    async fn fetched_engine(&self, encoding: Encoding) -> Result<Arc<TiktokenEngine>, FetchError> {
        match encoding {
            Encoding::R50kBase => {
                let data = self
                    .read(&self.source.r50k_url, &self.source.r50k_sha256)
                    .await?;
                Ok(Arc::new(TiktokenEngine::from_bpe_file(encoding, &data)?))
            }
            Encoding::Gpt2 => {
                let vocabulary = self
                    .read(
                        &self.source.gpt2_vocabulary_url,
                        &self.source.gpt2_vocabulary_sha256,
                    )
                    .await?;
                let encoder = self
                    .read(
                        &self.source.gpt2_encoder_url,
                        &self.source.gpt2_encoder_sha256,
                    )
                    .await?;
                let bpe = gpt2::bpe_file(&vocabulary, &encoder)?;
                Ok(Arc::new(TiktokenEngine::from_bpe_file(encoding, &bpe)?))
            }
            Encoding::Cl100kBase
            | Encoding::O200kBase
            | Encoding::P50kBase
            | Encoding::P50kEdit
            | Encoding::O200kHarmony => Err(FetchError::Engine(EngineError::NotBundled {
                encoding: encoding.name(),
            })),
        }
    }

    async fn read(&self, url: &str, expected_sha256: &str) -> Result<Arc<[u8]>, FetchError> {
        if let Some(found) = self
            .memory
            .lock()
            .ok()
            .and_then(|cache| cache.get(url).map(Arc::clone))
        {
            return Ok(found);
        }
        let bytes: Arc<[u8]> = match &self.cache_dir {
            Some(directory) => Arc::from(
                self.read_through_cache(directory, url, expected_sha256)
                    .await?,
            ),
            None => Arc::from(self.fetch(url, expected_sha256).await?),
        };
        if let Ok(mut cache) = self.memory.lock() {
            cache.insert(url.to_owned(), Arc::clone(&bytes));
        }
        Ok(bytes)
    }

    async fn read_through_cache(
        &self,
        directory: &Path,
        url: &str,
        expected_sha256: &str,
    ) -> Result<Vec<u8>, FetchError> {
        let key = hex::encode(Sha1::digest(url.as_bytes()));
        let path = directory.join(&key);
        match fs::read(&path).await {
            Ok(data) => {
                if hex::encode(Sha256::digest(&data)) == expected_sha256 {
                    return Ok(data);
                }
                let _removed = fs::remove_file(&path).await;
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(FetchError::CacheRead { path, source }),
        }
        let data = self.fetch(url, expected_sha256).await?;
        fs::create_dir_all(directory)
            .await
            .map_err(|source| FetchError::CacheWrite {
                path: path.clone(),
                source,
            })?;
        let temporary = directory.join(format!("{key}.{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, &data)
            .await
            .map_err(|source| FetchError::CacheWrite {
                path: temporary.clone(),
                source,
            })?;
        fs::rename(&temporary, &path)
            .await
            .map_err(|source| FetchError::CacheWrite {
                path: path.clone(),
                source,
            })?;
        Ok(data)
    }

    async fn fetch(&self, url: &str, expected_sha256: &str) -> Result<Vec<u8>, FetchError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: Url::parse(url).map_err(FetchError::Url)?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: FETCH_TIMEOUT,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(FetchError::Request)?;
        if !(200..300).contains(&response.status) {
            return Err(FetchError::Status {
                url: url.to_owned(),
                status: response.status,
            });
        }
        if hex::encode(Sha256::digest(&response.body)) != expected_sha256 {
            return Err(FetchError::HashMismatch {
                url: url.to_owned(),
            });
        }
        Ok(response.body)
    }
}
