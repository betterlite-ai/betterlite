#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process;
use std::thread;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use url::Url;

use crate::loader::ConfigLoader;
use crate::proxy::{ProxyPolicy, ProxyRoute, ProxyWarning};

const PROXY: &str = "http://proxy.local:3128";
const DISABLED: &str = "litellm_settings: {disable_aiohttp_trust_env: true}";
const FALLBACK: &str =
    "litellm_settings: {disable_aiohttp_trust_env: true, aiohttp_trust_env: true}";

fn render(policy: &ProxyPolicy, target: &str) -> String {
    let url = Url::parse(target).unwrap();
    let route = match policy.route(&url) {
        None => "direct".to_owned(),
        Some(ProxyRoute::Forward(server)) => {
            let credentials = server.credentials.as_ref();
            let user = credentials
                .map(|found| format!("{}:{}@", found.username, found.password.expose_secret()));
            let (host, port) = (&server.host, server.port);
            format!("{}{host}:{port}", user.unwrap_or_default())
        }
        Some(ProxyRoute::Unsupported { scheme }) => format!("unsupported {scheme}"),
        Some(ProxyRoute::Invalid { name }) => format!("invalid {name}"),
    };
    let warnings = policy.warnings.iter().map(|warning| match warning {
        ProxyWarning::SecureProxyIgnored { name, scheme } => format!(" warn {name} {scheme}"),
        ProxyWarning::NetrcUnusable { .. } => " warn netrc".to_owned(),
    });
    format!("{route}{}", warnings.collect::<String>())
}

#[rstest]
#[case::on_by_default("", &[("HTTPS_PROXY", PROXY)], "https://api.example.com", "proxy.local:3128")]
#[case::trust_env_false_keeps_it_on("litellm_settings: {aiohttp_trust_env: false}", &[("HTTPS_PROXY", PROXY)], "https://api.example.com", "proxy.local:3128")]
#[case::off_by_setting(DISABLED, &[("HTTPS_PROXY", PROXY)], "https://api.example.com", "direct")]
#[case::off_by_environment("", &[("DISABLE_AIOHTTP_TRUST_ENV", " TRUE "), ("HTTPS_PROXY", PROXY)], "https://api.example.com", "direct")]
#[case::unrecognized_disable_value("", &[("DISABLE_AIOHTTP_TRUST_ENV", "1"), ("HTTPS_PROXY", PROXY)], "https://api.example.com", "proxy.local:3128")]
#[case::lowercase_wins("", &[("HTTPS_PROXY", "http://upper.local:1"), ("https_proxy", "http://lower.local:2")], "https://api.example.com", "lower.local:2")]
#[case::empty_lowercase_clears("", &[("HTTPS_PROXY", PROXY), ("https_proxy", "")], "https://api.example.com", "direct")]
#[case::cgi_drops_uppercase_http("", &[("HTTP_PROXY", PROXY), ("REQUEST_METHOD", "GET")], "http://api.example.com", "direct")]
#[case::cgi_keeps_lowercase_http("", &[("REQUEST_METHOD", "GET"), ("http_proxy", PROXY)], "http://api.example.com", "proxy.local:3128")]
#[case::no_proxy_suffix("", &[("HTTPS_PROXY", PROXY), ("NO_PROXY", "other.org, .Example.com")], "https://API.example.com", "direct")]
#[case::no_proxy_exact("", &[("HTTPS_PROXY", PROXY), ("no_proxy", "example.com")], "https://example.com", "direct")]
#[case::no_proxy_ipv6("", &[("HTTPS_PROXY", PROXY), ("NO_PROXY", "::1")], "https://[::1]:8443", "direct")]
#[case::no_proxy_ipv6_digit_tail_as_port("", &[("HTTPS_PROXY", PROXY), ("NO_PROXY", "fe80:")], "https://[fe80::1]", "direct")]
#[case::no_proxy_ipv6_hex_tail_kept("", &[("HTTPS_PROXY", PROXY), ("NO_PROXY", "fe80:")], "https://[fe80::a]", "proxy.local:3128")]
#[case::no_proxy_star("", &[("HTTPS_PROXY", PROXY), ("NO_PROXY", "*")], "https://api.example.com", "direct")]
#[case::no_proxy_other_host("", &[("HTTPS_PROXY", PROXY), ("NO_PROXY", "notexample.com")], "https://example.com", "proxy.local:3128")]
#[case::all_proxy_on_default_path("", &[("ALL_PROXY", "http://all.local:8080")], "https://api.example.com", "all.local:8080")]
#[case::scheme_proxy_beats_all("", &[("ALL_PROXY", "http://all.local:8080"), ("HTTPS_PROXY", PROXY)], "https://api.example.com", "proxy.local:3128")]
#[case::decided_per_scheme("", &[("HTTP_PROXY", PROXY)], "https://api.example.com", "direct")]
#[case::missing_scheme_prefixed("", &[("HTTPS_PROXY", "proxy.local:3128")], "https://api.example.com", "proxy.local:3128")]
#[case::default_port("", &[("HTTPS_PROXY", "http://proxy.local")], "https://api.example.com", "proxy.local:80")]
#[case::decoded_credentials("", &[("HTTPS_PROXY", "http://us%40er:p%40ss%3Aword@proxy.local:3128")], "https://api.example.com", "us@er:p@ss:word@proxy.local:3128")]
#[case::username_without_password("", &[("HTTPS_PROXY", "http://user@proxy.local:3128")], "https://api.example.com", "user:@proxy.local:3128")]
#[case::unsupported_scheme("", &[("HTTPS_PROXY", "socks5://user:secret@proxy.local:1080")], "https://api.example.com", "unsupported socks5")]
#[case::invalid_port("", &[("HTTPS_PROXY", "http://user:secret@proxy.local:99999")], "https://api.example.com", "invalid https")]
#[case::invalid_escape("", &[("HTTPS_PROXY", "http://user:%FF@proxy.local:3128")], "https://api.example.com", "invalid https")]
#[case::fallback_when_disabled(FALLBACK, &[("HTTP_PROXY", PROXY)], "http://api.example.com", "proxy.local:3128")]
#[case::fallback_by_environment(DISABLED, &[("AIOHTTP_TRUST_ENV", "true"), ("HTTP_PROXY", PROXY)], "http://api.example.com", "proxy.local:3128")]
#[case::fallback_ignores_all_proxy(FALLBACK, &[("ALL_PROXY", PROXY)], "https://api.example.com", "direct")]
#[case::fallback_respects_no_proxy(FALLBACK, &[("HTTP_PROXY", PROXY), ("NO_PROXY", "example.com")], "http://example.com", "direct")]
#[case::fallback_drops_https_proxy(FALLBACK, &[("HTTPS_PROXY", "https://user:secret@proxy.local:443")], "https://api.example.com", "direct warn https https")]
#[case::fallback_has_no_prefix(FALLBACK, &[("HTTP_PROXY", "proxy.local:3128")], "http://api.example.com", "unsupported proxy.local")]
#[case::fallback_reads_netrc(FALLBACK, &[("HTTP_PROXY", PROXY), ("NETRC", "~/.netrc")], "http://api.example.com", "netrc-user:netrc-pass@proxy.local:3128")]
#[case::fallback_netrc_default(FALLBACK, &[("HTTP_PROXY", "http://other.local:1"), ("NETRC", "~/.netrc")], "http://api.example.com", "fallback-user:fallback-pass@other.local:1")]
#[case::fallback_url_beats_netrc(FALLBACK, &[("HTTP_PROXY", "http://url:pass@proxy.local:3128"), ("NETRC", "~/.netrc")], "http://api.example.com", "url:pass@proxy.local:3128")]
#[case::fallback_home_netrc(FALLBACK, &[("HOME", "~"), ("HTTP_PROXY", PROXY)], "http://api.example.com", "netrc-user:netrc-pass@proxy.local:3128")]
#[case::fallback_missing_home_netrc(FALLBACK, &[("HOME", "/nonexistent"), ("HTTP_PROXY", PROXY)], "http://api.example.com", "proxy.local:3128")]
#[case::fallback_missing_named_netrc(FALLBACK, &[("HTTP_PROXY", PROXY), ("NETRC", "/nonexistent/netrc")], "http://api.example.com", "proxy.local:3128 warn netrc")]
#[case::fallback_invalid_netrc(FALLBACK, &[("HTTP_PROXY", PROXY), ("NETRC", "~/invalid")], "http://api.example.com", "proxy.local:3128 warn netrc")]
fn http_proxies_proxy_policy_tests_behavior(
    #[case] settings: &str,
    #[case] variables: &[(&str, &str)],
    #[case] target: &str,
    #[case] expected: &str,
) {
    let case = thread::current().name().unwrap().replace("::", "-");
    let home = env::temp_dir().join(format!("betterlite-proxy-{}-{case}", process::id()));
    fs::create_dir_all(&home).unwrap();
    let netrc = "machine proxy.local login netrc-user password netrc-pass\ndefault login fallback-user password fallback-pass\n";
    fs::write(home.join(".netrc"), netrc).unwrap();
    fs::write(home.join("invalid"), "machine").unwrap();
    let home_text = home.display().to_string();
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.replace('~', &home_text)))
        .collect();
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(settings)
    .unwrap();
    let policy = ProxyPolicy::resolve(&config.litellm_settings.proxy_trust, &environment);
    fs::remove_dir_all(&home).unwrap();
    assert_eq!(render(&policy, target), expected);
    assert!(!format!("{policy:?}").contains("secret"));
}
