use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::limits::{ConvertJob, CountingService};
use crate::engine::tiktoken::Encoding;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use url::Url;
use uuid::Uuid;
pub mod gpt2;
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
    #[error("the tokenizer cache at {0} could not be read or written")]
    Cache(PathBuf, #[source] IoError),
    #[error(transparent)]
    Engine(#[from] EngineError),
}
enum Vocabulary {
    Tiktoken(Vec<u8>),
    Gpt2 {
        vocabulary: Vec<u8>,
        encoder: Vec<u8>,
    },
}
// tiktoken writes fetched files into TIKTOKEN_CACHE_DIR; betterlite converts them on the blocking facility and keeps the converted vocabulary in memory, with a cache dir also on disk under the sha1 of its URL
pub struct TiktokenFetcher {
    client: Arc<ProxiedClient>,
    cache_dir: Option<PathBuf>,
    base: String,
    r50k_sha256: String,
    gpt2_vocab_sha256: String,
    gpt2_encoder_sha256: String,
    memory: Mutex<BTreeMap<Encoding, Vec<u8>>>,
    counter: Arc<CountingService>,
}
impl TiktokenFetcher {
    #[must_use]
    pub fn new(
        client: Arc<ProxiedClient>,
        cache_dir: Option<PathBuf>,
        counter: Arc<CountingService>,
    ) -> Self {
        Self {
            client,
            cache_dir,
            base: "https://openaipublic.blob.core.windows.net".to_owned(),
            r50k_sha256: R50K_SHA256.to_owned(),
            gpt2_vocab_sha256: GPT2_VOCAB_SHA256.to_owned(),
            gpt2_encoder_sha256: GPT2_ENCODER_SHA256.to_owned(),
            memory: Mutex::new(BTreeMap::new()),
            counter,
        }
    }
    #[must_use]
    pub fn with_base(mut self, base: String, hashes: [String; 3]) -> Self {
        self.base = base;
        let [r50k, gpt2_vocab, gpt2_encoder] = hashes;
        self.r50k_sha256 = r50k;
        self.gpt2_vocab_sha256 = gpt2_vocab;
        self.gpt2_encoder_sha256 = gpt2_encoder;
        self
    }
    pub async fn data(&self, encoding: Encoding) -> Result<Vec<u8>, FetchError> {
        if let Some(bundled) = assets::bundled(encoding) {
            return Ok(bundled);
        }
        if let Some(found) = self
            .memory
            .lock()
            .ok()
            .and_then(|cache| cache.get(&encoding).cloned())
        {
            return Ok(found);
        }
        let files = self.remote(encoding).await?;
        let job: ConvertJob = Box::new(move || match files {
            Vocabulary::Tiktoken(data) => Ok(data),
            Vocabulary::Gpt2 {
                vocabulary,
                encoder,
            } => Ok(gpt2::bpe_file(&vocabulary, &encoder)?),
        });
        let data = self
            .counter
            .convert(job)
            .await
            .map_err(FetchError::Engine)?;
        if let Ok(mut cache) = self.memory.lock() {
            cache.insert(encoding, data.clone());
        }
        Ok(data)
    }
    async fn remote(&self, encoding: Encoding) -> Result<Vocabulary, FetchError> {
        if encoding == Encoding::R50kBase {
            let url = format!("{}/encodings/r50k_base.tiktoken", self.base);
            let data = self.read(&url, &self.r50k_sha256).await?;
            return Ok(Vocabulary::Tiktoken(data));
        }
        let vocabulary_url = format!("{}/gpt-2/encodings/main/vocab.bpe", self.base);
        let encoder_url = format!("{}/gpt-2/encodings/main/encoder.json", self.base);
        let vocabulary = self.read(&vocabulary_url, &self.gpt2_vocab_sha256).await?;
        let encoder = self.read(&encoder_url, &self.gpt2_encoder_sha256).await?;
        Ok(Vocabulary::Gpt2 {
            vocabulary,
            encoder,
        })
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
            Ok(data) if hex::encode(Sha256::digest(&data)) == expected => return Ok(data),
            Ok(_) => {
                let _removed = fs::remove_file(&path).await;
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(FetchError::Cache(path, source)),
        }
        let data = self.fetch(url, expected).await?;
        fs::create_dir_all(directory)
            .await
            .map_err(|source| FetchError::Cache(path.clone(), source))?;
        let temporary = directory.join(format!("{key}.{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, &data)
            .await
            .map_err(|source| FetchError::Cache(temporary.clone(), source))?;
        fs::rename(&temporary, &path)
            .await
            .map_err(|source| FetchError::Cache(path, source))?;
        Ok(data)
    }
    async fn fetch(&self, url: &str, expected: &str) -> Result<Vec<u8>, FetchError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: Url::parse(url).map_err(FetchError::Url)?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: Duration::from_secs(600),
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
