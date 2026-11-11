use std::collections::VecDeque;
use std::future::pending;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, ECDSA_P384_SHA384_FIXED_SIGNING, EcdsaKeyPair,
    EcdsaVerificationAlgorithm, KeyPair as _, UnparsedPublicKey,
};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sections::gdc_settings::{BoolOrString, CallSettings, GlobalSettings};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use tokio::task::yield_now;

use crate::gdc_auth::authenticator::{GdcAuth, GdcAuthParts, GdcValidation};
use crate::gdc_auth::cache::TokenCache;
use crate::gdc_auth::credential::{FixtureCredentials, GdchCredentialFactory, PemCredentials};
use crate::gdc_auth::headers::Headers;
use crate::gdc_auth::key::{GdcKey, GdchDescriptor, decode_gdc_key};
use crate::gdc_auth::transport::{
    CaCertificates, CaError, GdcClock, GdcJitter, GdcSleeper, TokenExchangeRequest,
    TokenExchangeResponse, TokenTransport, TokenTrust, TransportFailure,
};

pub(super) const TOKEN_URI: &str = "https://identity.example/authenticate";
pub(super) const FIXTURE_TOKEN: &str = "fixture-token";

#[rustfmt::skip]
pub(super) const P256_SEC1: &str = "-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIAr226GhVb4HXXWcEnZpKvTI8DSdZd1/TO7O2Tze+4idoAoGCCqGSM49
AwEHoUQDQgAENwjGbVLIZq9APsa6EvoyFi/jV6OPzkS4hWJr/ye6ZtHbgyHvjrjK
uGnYDLCN72yxSWscuQbZJJS+WLpyqFBKKw==
-----END EC PRIVATE KEY-----
";

#[rustfmt::skip]
pub(super) const P256_PKCS8: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n",
    "MIG", "HAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgCvbboaFVvgdddZwS\n",
    "dmkq9MjwNJ1l3X9M7s7ZPN77iJ2hRANCAAQ3CMZtUshmr0A+xroS+jIWL+NXo4/O\n",
    "RLiFYmv/J7pm0duDIe+OuMq4adgMsI3vbLFJaxy5BtkklL5YunKoUEor\n",
    "-----END PRIVATE KEY-----\n",
);

#[rustfmt::skip]
pub(super) const P384_SEC1: &str = "-----BEGIN EC PRIVATE KEY-----
MIGkAgEBBDDfj1eEDlvAUlN44J4aCLOGNZI8r4DabYY7X0AOvy2SKO3UUKWjehy3
dzjzMBkvJR6gBwYFK4EEACKhZANiAASeOhdRYeTjX1RLkBzQwmq79NnxaQxD/Ssn
xiF3/spFOEZDnDWyaaooCJNnbRPoDSEby8ZWi84ZbIWP/U/AGJNq5SKFDPpBUSv1
69vS+vbPZRZN3tyJcYAhgaPcottV6DU=
-----END EC PRIVATE KEY-----
";

#[rustfmt::skip]
pub(super) const P384_PKCS8: &str = "-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDDfj1eEDlvAUlN44J4a
CLOGNZI8r4DabYY7X0AOvy2SKO3UUKWjehy3dzjzMBkvJR6hZANiAASeOhdRYeTj
X1RLkBzQwmq79NnxaQxD/SsnxiF3/spFOEZDnDWyaaooCJNnbRPoDSEby8ZWi84Z
bIWP/U/AGJNq5SKFDPpBUSv169vS+vbPZRZN3tyJcYAhgaPcottV6DU=
-----END PRIVATE KEY-----
";

#[rustfmt::skip]
pub(super) const P521_SEC1: &str = "-----BEGIN EC PRIVATE KEY-----
MIHcAgEBBEIAbWBbSo2M9c68SkdltEhNZvktxmPaDVpFuM3sMVoQsrAw6Q4avFag
2bxqgh94+rdpCUl3eed/mhun+/HhgRijP5egBwYFK4EEACOhgYkDgYYABAF0lilS
HE4nBPV5f4w9wenGoHfuxjdYqp9mmr7OM5OqOf46n2TaI8eQHcLLO3/9tSqtwMx5
yYCppZ0xzKpjM+YOYQFxOoX0MMvCHbmvXd9sjX71oquQR6kTtVWpkSGffEedM8zX
vGVJOpTqA+h+4RwiNv3NTXcGtVWReP1RsIMZzcfrwg==
-----END EC PRIVATE KEY-----
";

#[rustfmt::skip]
pub(super) const GARBAGE_PEM: &str = "-----BEGIN EC PRIVATE KEY-----
bm90IGEga2V5
-----END EC PRIVATE KEY-----
";

#[rustfmt::skip]
pub(super) const RSA_PRIVATE_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----
MIICXgIBAAKBgQC0XIdyE4Z5hpHxvFsm9FS4PlKw1Tb63mXiaE/iVuxZAj226wS1
1Hs0NdMQRwgRyi409x28PxCRL04D10XmZkdKgDqGRbnSBo+3aKfwPF0NeZ3bsABF
rRCKzCED8eMj82O9PQlUaq2jubjflS7yMFb8qqEOJiJ9n9soATWiUI/egwIDAQAB
AoGATmRtRorwBwATjRNGNy85Z39HJ670n6NsezTFi+YmAiX0Nv39ZR6+khnW7BtD
iZ/r53+FAB4VEFWUPh0iN5s63o3cCguac5DmaDlfDqyCN3q87ihgCd6oZzh9BMme
NXPBqzRPGlJbDlegA2RhysYbWe4EUtW/FZQBJ1gdlU9UYpECQQDwLlajfvGnzixE
wXLIvQmAqHMxBgRikHE3/a6OS80rzzQztlX2+EcvLGavb9mqdLmBhlMOkPgGC/Sr
651oV6gJAkEAwD2Um6NOV9qnEL8CSUKsua0ZJTiEHQy7hQ4s3C5nakrJa7sq8YRE
Z4ym31GoTQefKwaAg1EJML4aj+Lg3oi9KwJBALbWb7nyzz+xId4NbbYGwg8hLN+q
9veZgbbf9Vv5tGyfRdv/+B0UEAyHPnxbJ1YtmXJSsTOrhEQGIfImYSOnZxECQQC+
kAyxH3f4pLm5Yu7cOmjIAiwL9nMX7Ngx9DRJ6wH+nrX9weK22NuKHiYI94QzH5wL
ZX2dLoZCuB/ShB9O2RNTAkEA4GzqRikrK1fO55phRdsCZo5sNNCTGHbpY/ZvVynz
2oz8MLv9HNG1ZMYtpi8z6ANXf/be/Nho3gLTi9c2aIffQA==
-----END RSA PRIVATE KEY-----
";

#[rustfmt::skip]
pub(super) const RSA_PKCS8_PEM: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n",
    "MIICeAIBADANBgkqhkiG9w0BAQEFAASCAmIwggJeAgEAAoGBALRch3IThnmGkfG8\n",
    "Wyb0VLg+UrDVNvreZeJoT+JW7FkCPbbrBLXUezQ10xBHCBHKLjT3Hbw/EJEvTgPX\n",
    "ReZmR0qAOoZFudIGj7dop/A8XQ15nduwAEWtEIrMIQPx4yPzY709CVRqraO5uN+V\n",
    "LvIwVvyqoQ4mIn2f2ygBNaJQj96DAgMBAAECgYBOZG1G", "ivAHABONE0Y3Lzlnf0cn\n",
    "rvSfo2x7NMWL5iYCJfQ2/f1lHr6SGdbsG0OJn+vnf4UAHhUQVZQ+HSI3mzrejdwK\n",
    "C5pzkOZoOV8OrII3erzuKGAJ3qhnOH0EyZ41c8GrNE8aUlsOV6ADZGHKxhtZ7gRS\n",
    "1b8VlAEnWB2VT1RikQJBAPAuVqN+8afOLETBcsi9CYCoczEGBGKQcTf9ro5LzSvP\n",
    "NDO2Vfb4Ry8sZq9v2ap0uYGGUw6Q+AYL9KvrnWhXqAkCQQDAPZSbo05X2qcQvwJJ\n",
    "Qqy5rRklOIQdDLuFDizcLmdqSslruyrxhERnjKbfUahNB58rBoCDUQkwvhqP4uDe\n",
    "iL0rAkEAttZvufLPP7Eh3g1ttgbCDyEs36r295mBtt/1W/m0bJ9F2//4HRQQDIc+\n",
    "fFsnVi2ZclKxM6uERAYh8iZhI6dnEQJBAL6QDLEfd/ikubli7tw6aMgCLAv2cxfs\n",
    "2DH0NEnrAf6etf3B4rbY24oeJgj3hDMfnAtlfZ0uhkK4H9KEH07ZE1MCQQDgbOpG\n",
    "KSsrV87nmmFF2wJmjmw00JMYdulj9m9XKfPajPwwu/0c0bVkxi2mLzPoA1d/9t78\n",
    "2GjeAtOL1zZoh99A\n",
    "-----END PRIVATE KEY-----\n",
);

pub(super) struct FakeClock {
    seconds: AtomicI64,
    millis: AtomicU64,
}

impl FakeClock {
    pub(super) fn at(seconds: i64) -> Arc<Self> {
        Arc::new(Self {
            seconds: AtomicI64::new(seconds),
            millis: AtomicU64::new(0),
        })
    }

    pub(super) fn advance_seconds(&self, seconds: i64) {
        self.seconds.fetch_add(seconds, Ordering::SeqCst);
    }

    pub(super) fn advance_millis(&self, millis: u64) {
        self.millis.fetch_add(millis, Ordering::SeqCst);
    }
}

impl GdcClock for FakeClock {
    fn unix_seconds(&self) -> i64 {
        self.seconds.load(Ordering::SeqCst)
    }

    fn monotonic_millis(&self) -> u64 {
        self.millis.load(Ordering::SeqCst)
    }
}

pub(super) struct FakeSleeper {
    clock: Arc<FakeClock>,
    waits: Mutex<Vec<u64>>,
}

impl FakeSleeper {
    pub(super) fn new(clock: Arc<FakeClock>) -> Self {
        Self {
            clock,
            waits: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn waits(&self) -> Vec<u64> {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl GdcSleeper for FakeSleeper {
    async fn sleep(&self, delay: Duration) {
        let millis = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(millis);
        self.clock.advance_millis(millis);
    }
}

#[derive(Default)]
pub(super) struct RangeJitter {
    ranges: Mutex<Vec<(u64, u64)>>,
}

impl RangeJitter {
    pub(super) fn ranges(&self) -> Vec<(u64, u64)> {
        self.ranges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GdcJitter for RangeJitter {
    fn uniform_millis(&self, low_inclusive: u64, high_inclusive: u64) -> u64 {
        self.ranges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((low_inclusive, high_inclusive));
        low_inclusive
    }
}

pub(super) enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Failure,
    Pending,
    Yield,
}

#[derive(Clone)]
pub(super) struct ExchangeRecord {
    pub(super) url: String,
    pub(super) body: String,
    pub(super) trust: TokenTrust,
    pub(super) timeout: Duration,
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    records: Mutex<Vec<ExchangeRecord>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            records: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn records(&self) -> Vec<ExchangeRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn count(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl TokenTransport for ScriptedTransport {
    async fn exchange(
        &self,
        request: TokenExchangeRequest,
    ) -> Result<TokenExchangeResponse, TransportFailure> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ExchangeRecord {
                url: request.url,
                body: request.body,
                trust: request.trust,
                timeout: request.timeout,
            });
        loop {
            let reply = self
                .replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            match reply {
                Some(Reply::Answer { status, body }) => {
                    return Ok(TokenExchangeResponse { status, body });
                }
                Some(Reply::Failure) | None => {
                    return Err(TransportFailure::Request {
                        source: Box::new(TransportError::Closed),
                    });
                }
                Some(Reply::Pending) => pending::<()>().await,
                Some(Reply::Yield) => yield_now().await,
            }
        }
    }
}

#[derive(Default)]
pub(super) struct RecordingCa {
    pub(super) bytes: Vec<u8>,
    pub(super) paths: Mutex<Vec<String>>,
}

impl RecordingCa {
    pub(super) fn paths(&self) -> Vec<String> {
        self.paths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CaCertificates for RecordingCa {
    async fn load(&self, path: &str) -> Result<Vec<u8>, CaError> {
        self.paths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.to_owned());
        Ok(self.bytes.clone())
    }
}

pub(super) struct Harness {
    pub(super) clock: Arc<FakeClock>,
    pub(super) transport: Arc<ScriptedTransport>,
    pub(super) ca: Arc<RecordingCa>,
    pub(super) auth: GdcAuth,
}

impl Harness {
    pub(super) fn fixture(replies: Vec<Reply>) -> Self {
        Self::build(
            Arc::new(FixtureCredentials {
                token: SecretString::from(FIXTURE_TOKEN.to_owned()),
            }),
            replies,
        )
    }

    pub(super) fn pem(replies: Vec<Reply>) -> Self {
        Self::build(Arc::new(PemCredentials), replies)
    }

    fn build(credentials: Arc<dyn GdchCredentialFactory>, replies: Vec<Reply>) -> Self {
        let clock = FakeClock::at(1_700_000_000);
        let transport = Arc::new(ScriptedTransport::new(replies));
        let jitter = Arc::new(RangeJitter::default());
        let sleeper = Arc::new(FakeSleeper::new(Arc::clone(&clock)));
        let ca = Arc::new(RecordingCa::default());
        let transport_clone = Arc::clone(&transport);
        let transport_part: Arc<dyn TokenTransport> = transport_clone;
        let ca_clone = Arc::clone(&ca);
        let ca_part: Arc<dyn CaCertificates> = ca_clone;
        let clock_clone = Arc::clone(&clock);
        let clock_part: Arc<dyn GdcClock> = clock_clone;
        let sleeper_clone = Arc::clone(&sleeper);
        let sleeper_part: Arc<dyn GdcSleeper> = sleeper_clone;
        let jitter_clone = Arc::clone(&jitter);
        let jitter_part: Arc<dyn GdcJitter> = jitter_clone;
        let auth = GdcAuth {
            parts: GdcAuthParts {
                credentials,
                transport: transport_part,
                ca: ca_part,
                clock: clock_part,
                sleeper: sleeper_part,
                jitter: jitter_part,
            },
            cache: TokenCache::default(),
        };
        Self {
            clock,
            transport,
            ca,
            auth,
        }
    }
}

#[rustfmt::skip]
#[derive(serde::Serialize)]
struct DescriptorJson<'value> {
    #[serde(rename = "type")] kind: &'value str,
    format_version: &'value str,
    private_key_id: &'value str,
    private_key: &'value str,
    name: &'value str,
    project: &'value str,
    token_uri: &'value str,
    #[serde(skip_serializing_if = "Option::is_none")] ca_cert_path: Option<&'value str>,
}

fn descriptor_text(project: &str, pem: &str, ca_cert_path: Option<&str>) -> String {
    serde_json::to_string(&DescriptorJson {
        kind: "gdch_service_account",
        format_version: "1",
        private_key_id: "key-id",
        private_key: pem,
        name: "service-identity",
        project,
        token_uri: TOKEN_URI,
        ca_cert_path,
    })
    .expect("a fixture descriptor")
}

pub(super) fn descriptor_json(project: &str, pem: &str) -> String {
    descriptor_text(project, pem, None)
}

pub(super) fn descriptor_with_ca(project: &str, pem: &str, ca_cert_path: &str) -> String {
    descriptor_text(project, pem, Some(ca_cert_path))
}

pub(super) fn descriptor(project: &str, pem: &str) -> GdchDescriptor {
    match decode_gdc_key(descriptor_json(project, pem).as_bytes(), "gdc/fixture") {
        Ok(GdcKey::ServiceAccount(descriptor)) => descriptor,
        Ok(GdcKey::Bearer(_)) | Err(_) => panic!("a fixture descriptor"),
    }
}

pub(super) fn token_reply(expires_in: Option<&str>) -> Vec<u8> {
    expires_in
        .map_or_else(
            || "{\"access_token\":\"exchanged-token\"}".to_owned(),
            |seconds| format!("{{\"access_token\":\"exchanged-token\",\"expires_in\":{seconds}}}"),
        )
        .into_bytes()
}

pub(super) fn answer(body: Vec<u8>) -> Reply {
    Reply::Answer { status: 200, body }
}

pub(super) struct Request {
    pub(super) api_key: SecretString,
    pub(super) base: String,
    pub(super) call: CallSettings,
    pub(super) global: GlobalSettings,
    pub(super) optional_params: Object,
    pub(super) headers: Headers,
    pub(super) ssl_verify: BoolOrString,
    pub(super) caching: bool,
    pub(super) deadline: Duration,
    pub(super) source_identity: String,
}

impl Request {
    pub(super) fn new(api_key: &str, base: &str) -> Self {
        Self {
            api_key: SecretString::from(api_key.to_owned()),
            base: base.to_owned(),
            call: CallSettings::default(),
            global: GlobalSettings::default(),
            optional_params: Object::new(),
            headers: Headers::new(),
            ssl_verify: BoolOrString::Bool(true),
            caching: false,
            deadline: Duration::from_secs(30),
            source_identity: "source-handle".to_owned(),
        }
    }

    pub(super) fn validation(&self) -> GdcValidation<'_> {
        GdcValidation {
            model: "gdc/gemini-2.5-pro",
            api_base: Some(&self.base),
            api_key: Some(&self.api_key),
            headers: self.headers.clone(),
            optional_params: &self.optional_params,
            call: &self.call,
            global: &self.global,
            ssl_verify: self.ssl_verify.clone(),
            caching: self.caching,
            deadline: self.deadline,
            source_identity: &self.source_identity,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct JwtHeader {
    pub(super) typ: Option<String>,
    pub(super) alg: String,
    pub(super) kid: Option<String>,
}

#[rustfmt::skip]
#[derive(Debug, serde::Deserialize)]
pub(super) struct JwtClaims {
    pub(super) iss: String,
    pub(super) sub: String,
    pub(super) aud: String,
    pub(super) iat: i64,
    pub(super) exp: i64,
}

fn jwt_segment(token: &str, index: usize) -> Vec<u8> {
    let segment = token.split('.').nth(index).expect("a JWT segment");
    URL_SAFE_NO_PAD
        .decode(segment)
        .expect("a base64url segment")
}

pub(super) fn jwt_header(token: &str) -> JwtHeader {
    serde_json::from_slice(&jwt_segment(token, 0)).expect("a JWT header")
}

pub(super) fn jwt_claims(token: &str) -> JwtClaims {
    serde_json::from_slice(&jwt_segment(token, 1)).expect("JWT claims")
}

pub(super) fn jwt_signing_parts(token: &str) -> (Vec<u8>, Vec<u8>) {
    let mut segments = token.split('.');
    let header = segments.next().expect("a header segment");
    let claims = segments.next().expect("a claims segment");
    let signature = segments.next().expect("a signature segment");
    (
        format!("{header}.{claims}").into_bytes(),
        URL_SAFE_NO_PAD
            .decode(signature)
            .expect("a base64url signature"),
    )
}

pub(super) fn verify_ecdsa(
    pem_text: &str,
    algorithm: &'static EcdsaVerificationAlgorithm,
    token: &str,
) -> bool {
    let block = pem::parse(pem_text.as_bytes()).expect("a PEM fixture");
    let contents = block.into_contents();
    let parsed = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &contents)
        .or_else(|_error| EcdsaKeyPair::from_pkcs8(&ECDSA_P384_SHA384_FIXED_SIGNING, &contents));
    let Ok(signing_pair) = parsed else {
        return false;
    };
    let key = UnparsedPublicKey::new(algorithm, signing_pair.public_key().as_ref());
    let (input, signature) = jwt_signing_parts(token);
    key.verify(&input, &signature).is_ok()
}
