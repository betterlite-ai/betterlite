use crate::engine::assets;
use crate::engine::tiktoken::Encoding;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};
use std::num::TryFromIntError;
use std::path::{Path, PathBuf};
use std::str::{Utf8Error, from_utf8};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use url::Url;
use uuid::Uuid;
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
    Gpt2(#[from] Gpt2Error),
}
// tiktoken writes fetched files into TIKTOKEN_CACHE_DIR; betterlite keeps each fetched vocabulary in memory and, with a cache dir set, on disk under the sha1 of its URL
pub struct TiktokenFetcher {
    client: Arc<ProxiedClient>,
    cache_dir: Option<PathBuf>,
    base: String,
    r50k_sha256: String,
    gpt2_vocab_sha256: String,
    gpt2_encoder_sha256: String,
    memory: Mutex<BTreeMap<String, Vec<u8>>>,
}
impl TiktokenFetcher {
    #[must_use]
    pub fn new(client: Arc<ProxiedClient>, cache_dir: Option<PathBuf>) -> Self {
        Self {
            client,
            cache_dir,
            base: "https://openaipublic.blob.core.windows.net".to_owned(),
            r50k_sha256: R50K_SHA256.to_owned(),
            gpt2_vocab_sha256: GPT2_VOCAB_SHA256.to_owned(),
            gpt2_encoder_sha256: GPT2_ENCODER_SHA256.to_owned(),
            memory: Mutex::new(BTreeMap::new()),
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
        self.remote(encoding).await
    }
    async fn remote(&self, encoding: Encoding) -> Result<Vec<u8>, FetchError> {
        if encoding == Encoding::R50kBase {
            let url = format!("{}/encodings/r50k_base.tiktoken", self.base);
            return self.read(&url, &self.r50k_sha256).await;
        }
        let vocabulary_url = format!("{}/gpt-2/encodings/main/vocab.bpe", self.base);
        let encoder_url = format!("{}/gpt-2/encodings/main/encoder.json", self.base);
        let vocabulary = self.read(&vocabulary_url, &self.gpt2_vocab_sha256).await?;
        let encoder = self.read(&encoder_url, &self.gpt2_encoder_sha256).await?;
        Ok(bpe_file(&vocabulary, &encoder)?)
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
        if let Some(found) = self
            .memory
            .lock()
            .ok()
            .and_then(|cache| cache.get(url).cloned())
        {
            return Ok(found);
        }
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
        if let Ok(mut cache) = self.memory.lock() {
            cache.insert(url.to_owned(), response.body.clone());
        }
        Ok(response.body)
    }
}
#[derive(Debug, Error)]
pub enum Gpt2Error {
    #[error("the gpt2 merge file is not UTF-8")]
    VocabularyUtf8(#[source] Utf8Error),
    #[error("the gpt2 merge file has no version line")]
    MissingVersion,
    #[error("a gpt2 merge line does not hold exactly two tokens")]
    MalformedMerge,
    #[error("the gpt2 merge file uses a character outside its byte map")]
    UnknownCharacter,
    #[error("a gpt2 byte map character is not a Unicode scalar value")]
    BadCodepoint,
    #[error("the gpt2 encoder map is not valid JSON")]
    EncoderJson(#[source] serde_json::Error),
    #[error("a gpt2 token rank does not fit in 32 bits")]
    RankOverflow(#[source] TryFromIntError),
    #[error("the gpt2 vocabulary has too many tokens")]
    TooManyTokens,
}
// tiktoken's data_gym_to_mergeable_bpe_ranks; both files come sha256-checked, so the encoder map only has to parse
pub(crate) fn bpe_file(vocabulary: &[u8], encoder: &[u8]) -> Result<Vec<u8>, Gpt2Error> {
    let text = from_utf8(vocabulary).map_err(Gpt2Error::VocabularyUtf8)?;
    let mapping = byte_map()?;
    let parts: Vec<&str> = text.split('\n').collect();
    let end = parts
        .len()
        .checked_sub(1)
        .ok_or(Gpt2Error::MissingVersion)?;
    let merges = parts.get(1..end).ok_or(Gpt2Error::MissingVersion)?;
    let _encoder: BTreeMap<String, u32> =
        serde_json::from_slice(encoder).map_err(Gpt2Error::EncoderJson)?;
    Ok(bpe_lines(&tokens_from_merges(merges, &mapping)?))
}
fn tokens_from_merges(
    merges: &[&str],
    mapping: &BTreeMap<char, u8>,
) -> Result<Vec<(Vec<u8>, u32)>, Gpt2Error> {
    let ordered = rank_order();
    let mut tokens: Vec<(Vec<u8>, u32)> =
        Vec::with_capacity(ordered.len().saturating_add(merges.len()));
    for (rank, byte) in ordered.iter().enumerate() {
        tokens.push((
            vec![*byte],
            u32::try_from(rank).map_err(Gpt2Error::RankOverflow)?,
        ));
    }
    let mut next = u32::try_from(ordered.len()).map_err(Gpt2Error::RankOverflow)?;
    for merge in merges {
        let fields: Vec<&str> = merge.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(Gpt2Error::MalformedMerge);
        }
        let (Some(first), Some(second)) = (fields.first(), fields.get(1)) else {
            return Err(Gpt2Error::MalformedMerge);
        };
        let mut piece = Vec::new();
        for character in first.chars().chain(second.chars()) {
            piece.push(*mapping.get(&character).ok_or(Gpt2Error::UnknownCharacter)?);
        }
        tokens.push((piece, next));
        next = next.checked_add(1).ok_or(Gpt2Error::TooManyTokens)?;
    }
    Ok(tokens)
}
fn bpe_lines(tokens: &[(Vec<u8>, u32)]) -> Vec<u8> {
    let mut output = Vec::with_capacity(tokens.len().saturating_mul(32));
    for (bytes, rank) in tokens {
        output.extend_from_slice(STANDARD.encode(bytes).as_bytes());
        output.push(b' ');
        output.extend_from_slice(rank.to_string().as_bytes());
        output.push(b'\n');
    }
    output
}
const PRINTABLE: fn(u8) -> bool =
    |byte: u8| byte.is_ascii_graphic() || ((0xA1..=0xFF).contains(&byte) && byte != 0xAD);
fn rank_order() -> Vec<u8> {
    let (mut ordered, rest): (Vec<u8>, Vec<u8>) = (0_u8..=255).partition(|byte| PRINTABLE(*byte));
    ordered.extend(rest);
    ordered
}
fn byte_map() -> Result<BTreeMap<char, u8>, Gpt2Error> {
    let mut mapping = BTreeMap::new();
    for byte in (0_u8..=255).filter(|byte| PRINTABLE(*byte)) {
        mapping.insert(char::from(byte), byte);
    }
    let mut extra = 0_u32;
    for byte in (0_u8..=255).filter(|byte| !PRINTABLE(*byte)) {
        let codepoint = 256_u32.checked_add(extra).ok_or(Gpt2Error::BadCodepoint)?;
        mapping.insert(
            char::from_u32(codepoint).ok_or(Gpt2Error::BadCodepoint)?,
            byte,
        );
        extra = extra.checked_add(1).ok_or(Gpt2Error::BadCodepoint)?;
    }
    Ok(mapping)
}
