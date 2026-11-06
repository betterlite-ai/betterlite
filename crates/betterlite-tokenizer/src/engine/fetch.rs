use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::tiktoken::{Encoding, TiktokenEngine};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use url::Url;
use uuid::Uuid;
pub mod gpt2;
const TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_BASE: &str = "https://openaipublic.blob.core.windows.net";
const R50K_SHA256: &str = "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930";
const GPT2_VOCAB_SHA256: &str = "1ce1664773c50f3e0cc8842619a93edc4624525b728b188a9e0be33b7726adc5";
const GPT2_ENCODER_SHA256: &str =
    "196139668be63f3b5d6574427317ae82f612a97c5d1cdaf36ed2256dbf636783";
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("the tokenizer URL is invalid")]
    Url(#[source] url::ParseError),
    #[error("the tokenizer file could not be fetched")]
    Request(#[source] TransportError),
    #[error("fetching {0} returned status {1}")]
    Status(String, u16),
    #[error("the downloaded file {0} does not match its sha256")]
    HashMismatch(String),
    #[error("the tokenizer cache at {0} could not be read")]
    CacheRead(PathBuf, #[source] IoError),
    #[error("the tokenizer cache at {0} could not be written")]
    CacheWrite(PathBuf, #[source] IoError),
    #[error("the gpt2 vocabulary could not be converted")]
    Gpt2(#[from] gpt2::Gpt2Error),
    #[error(transparent)]
    Engine(#[from] EngineError),
}
// tiktoken writes fetched files into TIKTOKEN_CACHE_DIR; betterlite keeps them in memory unless an operator sets a cache
pub struct TiktokenFetcher {
    client: Arc<ProxiedClient>,
    cache_dir: Option<PathBuf>,
    base: String,
    r50k_sha256: String,
}
impl TiktokenFetcher {
    #[must_use]
    pub fn new(client: Arc<ProxiedClient>, cache_dir: Option<PathBuf>) -> Self {
        Self {
            client,
            cache_dir,
            base: DEFAULT_BASE.to_owned(),
            r50k_sha256: R50K_SHA256.to_owned(),
        }
    }
    #[must_use]
    pub fn with_base(mut self, base: String, r50k_sha256: String) -> Self {
        self.base = base;
        self.r50k_sha256 = r50k_sha256;
        self
    }
    pub async fn engine(&self, encoding: Encoding) -> Result<Arc<TiktokenEngine>, FetchError> {
        match assets::bundled(encoding) {
            Some(data) => Ok(Arc::new(TiktokenEngine::from_bpe_file(encoding, &data)?)),
            None => self.remote_engine(encoding).await,
        }
    }
    async fn remote_engine(&self, encoding: Encoding) -> Result<Arc<TiktokenEngine>, FetchError> {
        match encoding {
            Encoding::R50kBase => {
                let url = format!("{}/encodings/r50k_base.tiktoken", self.base);
                let data = self.read(&url, &self.r50k_sha256).await?;
                Ok(Arc::new(TiktokenEngine::from_bpe_file(encoding, &data)?))
            }
            Encoding::Gpt2 => {
                let vocabulary_url = format!("{}/gpt-2/encodings/main/vocab.bpe", self.base);
                let encoder_url = format!("{}/gpt-2/encodings/main/encoder.json", self.base);
                let vocabulary = self.read(&vocabulary_url, GPT2_VOCAB_SHA256).await?;
                let encoder = self.read(&encoder_url, GPT2_ENCODER_SHA256).await?;
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
    async fn read(&self, url: &str, expected: &str) -> Result<Vec<u8>, FetchError> {
        match &self.cache_dir {
            Some(directory) => self.cached(directory, url, expected).await,
            None => self.fetch(url, expected).await,
        }
    }
    async fn cached(
        &self,
        directory: &Path,
        url: &str,
        expected: &str,
    ) -> Result<Vec<u8>, FetchError> {
        let key = hex::encode(Sha1::digest(url.as_bytes()));
        let path = directory.join(&key);
        match fs::read(&path).await {
            Ok(data) => {
                if hex::encode(Sha256::digest(&data)) == expected {
                    return Ok(data);
                }
                let _removed = fs::remove_file(&path).await;
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(FetchError::CacheRead(path, source)),
        }
        let data = self.fetch(url, expected).await?;
        fs::create_dir_all(directory)
            .await
            .map_err(|source| FetchError::CacheWrite(path.clone(), source))?;
        let temporary = directory.join(format!("{key}.{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, &data)
            .await
            .map_err(|source| FetchError::CacheWrite(temporary.clone(), source))?;
        fs::rename(&temporary, &path)
            .await
            .map_err(|source| FetchError::CacheWrite(path, source))?;
        Ok(data)
    }
    async fn fetch(&self, url: &str, expected: &str) -> Result<Vec<u8>, FetchError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: Url::parse(url).map_err(FetchError::Url)?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: TIMEOUT,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(FetchError::Request)?;
        if !(200..300).contains(&response.status) {
            return Err(FetchError::Status(url.to_owned(), response.status));
        }
        if hex::encode(Sha256::digest(&response.body)) != expected {
            return Err(FetchError::HashMismatch(url.to_owned()));
        }
        Ok(response.body)
    }
}
