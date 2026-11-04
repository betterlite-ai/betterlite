use super::{BTreeMap, ClientBuilder, ConfigLoader, TlsPolicy, apply_policy};
use rstest::rstest;

#[rstest]
#[case::unreadable_ca("SSL_CERT_FILE", "missing-ca.pem", "SSL_CERT_FILE")]
#[case::invalid_header("LITELLM_USER_AGENT", "private\r\nheader", "LITELLM_USER_AGENT")]
#[case::unsupported_curve("SSL_ECDH_CURVE", "secp521r1", "SSL_ECDH_CURVE secp521r1")]
#[case::unsupported_x448("SSL_ECDH_CURVE", "X448", "SSL_ECDH_CURVE X448")]
#[case::unsupported_secp256k1("SSL_ECDH_CURVE", "secp256k1", "SSL_ECDH_CURVE secp256k1")]
#[case::unsupported_brainpool256(
    "SSL_ECDH_CURVE",
    "brainpoolP256r1",
    "SSL_ECDH_CURVE brainpoolP256r1"
)]
#[case::unsupported_brainpool384(
    "SSL_ECDH_CURVE",
    "brainpoolP384r1",
    "SSL_ECDH_CURVE brainpoolP384r1"
)]
#[case::unsupported_brainpool512(
    "SSL_ECDH_CURVE",
    "brainpoolP512r1",
    "SSL_ECDH_CURVE brainpoolP512r1"
)]
#[case::sect163k1("SSL_ECDH_CURVE", "sect163k1", "SSL_ECDH_CURVE sect163k1")]
#[case::sect163r1("SSL_ECDH_CURVE", "sect163r1", "SSL_ECDH_CURVE sect163r1")]
#[case::sect163r2("SSL_ECDH_CURVE", "sect163r2", "SSL_ECDH_CURVE sect163r2")]
#[case::sect193r1("SSL_ECDH_CURVE", "sect193r1", "SSL_ECDH_CURVE sect193r1")]
#[case::sect193r2("SSL_ECDH_CURVE", "sect193r2", "SSL_ECDH_CURVE sect193r2")]
#[case::sect233k1("SSL_ECDH_CURVE", "sect233k1", "SSL_ECDH_CURVE sect233k1")]
#[case::sect233r1("SSL_ECDH_CURVE", "sect233r1", "SSL_ECDH_CURVE sect233r1")]
#[case::sect239k1("SSL_ECDH_CURVE", "sect239k1", "SSL_ECDH_CURVE sect239k1")]
#[case::sect283k1("SSL_ECDH_CURVE", "sect283k1", "SSL_ECDH_CURVE sect283k1")]
#[case::sect283r1("SSL_ECDH_CURVE", "sect283r1", "SSL_ECDH_CURVE sect283r1")]
#[case::sect409k1("SSL_ECDH_CURVE", "sect409k1", "SSL_ECDH_CURVE sect409k1")]
#[case::sect409r1("SSL_ECDH_CURVE", "sect409r1", "SSL_ECDH_CURVE sect409r1")]
#[case::sect571k1("SSL_ECDH_CURVE", "sect571k1", "SSL_ECDH_CURVE sect571k1")]
#[case::sect571r1("SSL_ECDH_CURVE", "sect571r1", "SSL_ECDH_CURVE sect571r1")]
#[case::secp160k1("SSL_ECDH_CURVE", "secp160k1", "SSL_ECDH_CURVE secp160k1")]
#[case::secp160r1("SSL_ECDH_CURVE", "secp160r1", "SSL_ECDH_CURVE secp160r1")]
#[case::secp160r2("SSL_ECDH_CURVE", "secp160r2", "SSL_ECDH_CURVE secp160r2")]
#[case::secp192k1("SSL_ECDH_CURVE", "secp192k1", "SSL_ECDH_CURVE secp192k1")]
#[case::prime192v1("SSL_ECDH_CURVE", "prime192v1", "SSL_ECDH_CURVE prime192v1")]
#[case::secp224k1("SSL_ECDH_CURVE", "secp224k1", "SSL_ECDH_CURVE secp224k1")]
#[case::secp224r1("SSL_ECDH_CURVE", "secp224r1", "SSL_ECDH_CURVE secp224r1")]
#[case::ffdhe2048("SSL_ECDH_CURVE", "ffdhe2048", "SSL_ECDH_CURVE ffdhe2048")]
#[case::ffdhe3072("SSL_ECDH_CURVE", "ffdhe3072", "SSL_ECDH_CURVE ffdhe3072")]
#[case::ffdhe4096("SSL_ECDH_CURVE", "ffdhe4096", "SSL_ECDH_CURVE ffdhe4096")]
#[case::ffdhe6144("SSL_ECDH_CURVE", "ffdhe6144", "SSL_ECDH_CURVE ffdhe6144")]
#[case::ffdhe8192("SSL_ECDH_CURVE", "ffdhe8192", "SSL_ECDH_CURVE ffdhe8192")]
#[case::brainpoolp256r1tls13(
    "SSL_ECDH_CURVE",
    "brainpoolP256r1tls13",
    "SSL_ECDH_CURVE brainpoolP256r1tls13"
)]
#[case::brainpoolp384r1tls13(
    "SSL_ECDH_CURVE",
    "brainpoolP384r1tls13",
    "SSL_ECDH_CURVE brainpoolP384r1tls13"
)]
#[case::brainpoolp512r1tls13(
    "SSL_ECDH_CURVE",
    "brainpoolP512r1tls13",
    "SSL_ECDH_CURVE brainpoolP512r1tls13"
)]
fn http_policy_tests_behavior(#[case] key: &str, #[case] value: &str, #[case] named: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .unwrap();
    let environment = BTreeMap::from([(key.to_owned(), value.to_owned())]);
    let policy = TlsPolicy::resolve(&config, &environment, None).unwrap();
    let error = apply_policy(ClientBuilder::new(), &policy).unwrap_err();
    assert!(error.to_string().contains(named), "{error}");
    assert!(!format!("{error:?}").contains("private"));
}
