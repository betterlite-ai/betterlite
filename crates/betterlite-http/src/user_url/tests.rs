#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use reqwest::header::{AUTHORIZATION, COOKIE, HOST, HeaderMap, HeaderValue, LOCATION};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::proxy::limits::TransportLimits;
use crate::proxy::{ProxiedClient, base_builder};
use crate::user_url::address::is_blocked_text;
use crate::user_url::error::{InvalidUrl, UserUrlError};
use crate::user_url::fetch::UserUrlFetcher;
use crate::user_url::httpx_url::HttpxUrl;
use crate::user_url::join::urljoin;
use crate::user_url::redirect::{location, redirect_headers, redirect_url};
use crate::user_url::split::{Parsed, urlparse};
use crate::user_url::validate::{Resolve, SystemResolver, UrlValidator};

const HINT: &str = ". If this is a legitimate internal service, add the host to `user_url_allowed_hosts` in litellm_settings.";

// Names the tests rely on; any other name fails the way getaddrinfo does
struct FakeResolver;

#[async_trait]
impl Resolve for FakeResolver {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        if let Ok(address) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(address, port)]);
        }
        let addresses: &[&str] = match host {
            "public.test" => &["93.184.216.34"],
            "dual.test" => &["2606:2800:220:1::1", "93.184.216.34"],
            "mixed.test" => &["93.184.216.34", "10.0.0.5"],
            "internal.test" => &["10.1.2.3"],
            "app.test" => &["127.0.0.1"],
            "empty.test" => &[],
            _ => return Err(io::Error::other("Name or service not known")),
        };
        Ok(addresses
            .iter()
            .map(|text| SocketAddr::new(text.parse().unwrap(), port))
            .collect())
    }
}

#[rstest]
#[case::ftp_scheme("ftp://public.test/x", &[], Verification::Enabled, "ssrf: URL scheme 'ftp' is not allowed")]
#[case::scheme_missing("public.test/x", &[], Verification::Enabled, "ssrf: URL scheme '' is not allowed")]
#[case::no_hostname("http://", &[], Verification::Enabled, "ssrf: URL has no hostname")]
#[case::empty_host_with_path("http:///path", &[], Verification::Enabled, "ssrf: URL has no hostname")]
#[case::http_rewritten_to_first_address("http://public.test/a/b;p?q=1#frag", &[], Verification::Enabled, "http://93.184.216.34/a/b;p?q=1 | public.test")]
#[case::http_drops_userinfo_keeps_port("http://user:pw@public.test:8080/a?q=1#frag", &[], Verification::Enabled, "http://93.184.216.34:8080/a?q=1 | public.test:8080")]
#[case::http_default_port_stays_explicit("HTTP://PUBLIC.test:80/a", &[], Verification::Enabled, "http://93.184.216.34:80/a | public.test")]
#[case::https_kept_verbatim("https://user:pw@public.test:8443/a#frag", &[], Verification::Enabled, "https://user:pw@public.test:8443/a#frag | public.test:8443")]
#[case::https_kept_with_ca_file("https://public.test/a", &[], Verification::CaFile("/etc/ca.pem".to_owned()), "https://public.test/a | public.test")]
#[case::https_rewritten_without_verification("https://user:pw@public.test:8443/a#frag", &[], Verification::Disabled, "https://93.184.216.34:8443/a | public.test:8443")]
#[case::https_unverified_without_port("https://public.test/a", &[], Verification::Disabled, "https://93.184.216.34/a | public.test")]
#[case::https_unverified_default_port("https://public.test:443/a", &[], Verification::Disabled, "https://93.184.216.34:443/a | public.test")]
#[case::ipv6_first_address_bracketed("http://dual.test/a", &[], Verification::Enabled, "http://[2606:2800:220:1::1]/a | dual.test")]
#[case::ipv6_first_address_with_port("http://dual.test:81/a", &[], Verification::Enabled, "http://[2606:2800:220:1::1]:81/a | dual.test:81")]
#[case::one_blocked_address_blocks_all("http://mixed.test/a", &[], Verification::Enabled, "ssrf: URL targets a blocked address (10.0.0.5)<hint>")]
#[case::allowlist_skips_the_check_not_the_pin("http://mixed.test/a", &["mixed.test"], Verification::Enabled, "http://93.184.216.34/a | mixed.test")]
#[case::private_host_blocked("http://internal.test/a", &[], Verification::Enabled, "ssrf: URL targets a blocked address (10.1.2.3)<hint>")]
#[case::allowlist_entry_lowercased_and_trailing_dot_stripped("http://internal.test/a", &["INTERNAL.test."], Verification::Enabled, "http://10.1.2.3/a | internal.test")]
#[case::allowlist_with_matching_port("http://internal.test:8080/a", &["internal.test:8080"], Verification::Enabled, "http://10.1.2.3:8080/a | internal.test:8080")]
#[case::allowlist_port_mismatch("http://internal.test:8081/a", &["internal.test:8080"], Verification::Enabled, "ssrf: URL targets a blocked address (10.1.2.3)<hint>")]
#[case::allowlist_skips_empty_entries("http://internal.test/a", &["", "other.test"], Verification::Enabled, "ssrf: URL targets a blocked address (10.1.2.3)<hint>")]
#[case::allowlist_port_matches_default_port("http://internal.test/a", &["internal.test:80"], Verification::Enabled, "http://10.1.2.3/a | internal.test")]
#[case::no_addresses("http://empty.test/a", &[], Verification::Enabled, "ssrf: No addresses found for 'empty.test'")]
#[case::dns_failure("http://missing.test/a", &[], Verification::Enabled, "ssrf: DNS resolution failed for 'missing.test': Name or service not known")]
#[case::loopback_blocked("http://127.0.0.1/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (127.0.0.1)<hint>")]
#[case::loopback_allowlisted("http://127.0.0.1:8000/x", &["127.0.0.1"], Verification::Enabled, "http://127.0.0.1:8000/x | 127.0.0.1:8000")]
#[case::ipv6_loopback_blocked("http://[::1]/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (::1)<hint>")]
#[case::ipv6_allowlist_with_port("http://[::1]:9/x", &["[::1]:9"], Verification::Enabled, "http://[::1]:9/x | [::1]:9")]
#[case::mapped_loopback_blocked("http://[::ffff:127.0.0.1]/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (::ffff:127.0.0.1)<hint>")]
#[case::azure_wire_server_blocked("http://168.63.129.16/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (168.63.129.16)<hint>")]
#[case::ietf_protocol_block_blocked("http://192.0.0.8/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (192.0.0.8)<hint>")]
#[case::ietf_exception_inside_block_blocked("http://192.0.0.9/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (192.0.0.9)<hint>")]
#[case::shared_address_space_blocked("http://100.64.0.1/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (100.64.0.1)<hint>")]
#[case::documentation_ipv6_blocked("http://[2001:db8::1]/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (2001:db8::1)<hint>")]
#[case::multicast_blocked("http://224.0.0.1/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (224.0.0.1)<hint>")]
#[case::link_local_ipv6_blocked("http://[fe80::1]/", &[], Verification::Enabled, "ssrf: URL targets a blocked address (fe80::1)<hint>")]
#[case::port_out_of_range("http://public.test:99999/", &[], Verification::Enabled, "value: Port out of range 0-65535")]
#[case::port_not_a_number("http://public.test:abc/", &[], Verification::Enabled, "value: Port could not be cast to integer value as 'abc'")]
#[case::unclosed_bracket("http://[::1/", &[], Verification::Enabled, "value: Invalid IPv6 URL")]
#[case::text_after_bracket("http://[::1]x/", &[], Verification::Enabled, "value: Invalid IPv6 URL")]
#[case::ipvfuture_host_is_resolved_by_name("http://[v1.x]/", &[], Verification::Enabled, "ssrf: DNS resolution failed for 'v1.x': Name or service not known")]
#[case::ipv4_in_brackets("http://[1.2.3.4]/", &[], Verification::Enabled, "value: An IPv4 address cannot be in brackets")]
#[case::bracketed_non_address("http://[zzz]/", &[], Verification::Enabled, "value: 'zzz' does not appear to be an IPv4 or IPv6 address")]
#[case::ipvfuture_without_hex_digits("http://[v.x]/", &[], Verification::Enabled, "value: IPvFuture address is invalid")]
#[case::ipvfuture_with_empty_suffix("http://[v1.]/", &[], Verification::Enabled, "value: IPvFuture address is invalid")]
#[case::netloc_nfkc_introduces_separators("http://a\u{2100}.test/x", &[], Verification::Enabled, "value: netloc 'a\u{2100}.test' contains invalid characters under NFKC normalization")]
#[case::ipv6_https_allowlisted_without_port("https://[::1]:9/x", &["[::1]"], Verification::Disabled, "https://[::1]:9/x | [::1]:9")]
#[case::whitespace_and_line_breaks_removed("  http://public.test/\t\nabc", &[], Verification::Enabled, "http://93.184.216.34/abc | public.test")]
#[tokio::test]
async fn validate_url_matches_litellm(
    #[case] url: &str,
    #[case] allowed: &[&str],
    #[case] verification: Verification,
    #[case] expected: &str,
) {
    let allowed_hosts: Vec<String> = allowed.iter().map(|entry| (*entry).to_owned()).collect();
    let validator = UrlValidator::new(FakeResolver, &allowed_hosts, &verification);
    let observed = match validator.validate_url(url).await {
        Ok(validated) => format!("{} | {}", validated.url, validated.host_header),
        Err(UserUrlError::Ssrf(error)) => format!("ssrf: {error}"),
        Err(UserUrlError::Parse(error)) => format!("value: {error}"),
        Err(other) => format!("other: {other}"),
    };
    assert_eq!(observed, expected.replace("<hint>", HINT));
}

#[tokio::test]
async fn the_system_resolver_returns_literal_addresses() {
    let addresses = SystemResolver.resolve("127.0.0.1", 8080).await.unwrap();
    assert_eq!(addresses, vec!["127.0.0.1:8080".parse().unwrap()]);
}

#[rstest]
#[case::v4_0_0_0_0("0.0.0.0", true)]
#[case::v4_0_255_255_255("0.255.255.255", true)]
#[case::v4_1_0_0_0("1.0.0.0", false)]
#[case::v4_8_8_8_8("8.8.8.8", false)]
#[case::v4_9_255_255_255("9.255.255.255", false)]
#[case::v4_10_0_0_0("10.0.0.0", true)]
#[case::v4_11_0_0_0("11.0.0.0", false)]
#[case::v4_100_63_255_255("100.63.255.255", false)]
#[case::v4_100_64_0_0("100.64.0.0", true)]
#[case::v4_100_127_255_255("100.127.255.255", true)]
#[case::v4_100_128_0_0("100.128.0.0", false)]
#[case::v4_127_0_0_1("127.0.0.1", true)]
#[case::v4_126_255_255_255("126.255.255.255", false)]
#[case::v4_169_253_255_255("169.253.255.255", false)]
#[case::v4_169_254_169_254("169.254.169.254", true)]
#[case::v4_169_255_0_0("169.255.0.0", false)]
#[case::v4_172_15_255_255("172.15.255.255", false)]
#[case::v4_172_16_0_0("172.16.0.0", true)]
#[case::v4_172_31_255_255("172.31.255.255", true)]
#[case::v4_172_32_0_0("172.32.0.0", false)]
#[case::v4_192_0_0_0("192.0.0.0", true)]
#[case::v4_192_0_0_8("192.0.0.8", true)]
#[case::v4_192_0_0_9("192.0.0.9", true)]
#[case::v4_192_0_0_10("192.0.0.10", true)]
#[case::v4_192_0_0_11("192.0.0.11", true)]
#[case::v4_192_0_0_170("192.0.0.170", true)]
#[case::v4_192_0_0_171("192.0.0.171", true)]
#[case::v4_192_0_0_255("192.0.0.255", true)]
#[case::v4_192_0_1_0("192.0.1.0", false)]
#[case::v4_192_0_2_1("192.0.2.1", true)]
#[case::v4_192_88_99_1("192.88.99.1", false)]
#[case::v4_192_167_255_255("192.167.255.255", false)]
#[case::v4_192_168_0_1("192.168.0.1", true)]
#[case::v4_192_169_0_0("192.169.0.0", false)]
#[case::v4_198_17_255_255("198.17.255.255", false)]
#[case::v4_198_18_0_0("198.18.0.0", true)]
#[case::v4_198_19_255_255("198.19.255.255", true)]
#[case::v4_198_20_0_0("198.20.0.0", false)]
#[case::v4_198_51_100_7("198.51.100.7", true)]
#[case::v4_203_0_113_9("203.0.113.9", true)]
#[case::v4_223_255_255_255("223.255.255.255", false)]
#[case::v4_224_0_0_0("224.0.0.0", true)]
#[case::v4_239_255_255_255("239.255.255.255", true)]
#[case::v4_240_0_0_1("240.0.0.1", true)]
#[case::v4_255_255_255_255("255.255.255.255", true)]
#[case::v4_168_63_129_15("168.63.129.15", false)]
#[case::v4_168_63_129_16("168.63.129.16", true)]
#[case::v4_168_63_129_17("168.63.129.17", false)]
#[case::v6_empty("::", true)]
#[case::v6_1("::1", true)]
#[case::v6_2("::2", false)]
#[case::v6_ffff_0_0("::ffff:0:0", true)]
#[case::v6_ffff_8_8_8_8("::ffff:8.8.8.8", false)]
#[case::v6_ffff_10_0_0_1("::ffff:10.0.0.1", true)]
#[case::v6_ffff_168_63_129_16("::ffff:168.63.129.16", true)]
#[case::v6_ffff_0808_0808("::ffff:0808:0808", false)]
#[case::v6_1_2_3_4("::1.2.3.4", false)]
#[case::v6_64_ff9b_808_808("64:ff9b::808:808", false)]
#[case::v6_64_ff9b_1_1("64:ff9b:1::1", true)]
#[case::v6_100_1("100::1", true)]
#[case::v6_2001_1("2001::1", true)]
#[case::v6_2001_1_1("2001:1::1", false)]
#[case::v6_2001_1_2("2001:1::2", false)]
#[case::v6_2001_3_1("2001:3::1", false)]
#[case::v6_2001_4_112_1("2001:4:112::1", false)]
#[case::v6_2001_20_1("2001:20::1", false)]
#[case::v6_2001_30_1("2001:30::1", false)]
#[case::v6_2001_2_1("2001:2::1", true)]
#[case::v6_2001_200_1("2001:200::1", false)]
#[case::v6_2001_db8_1("2001:db8::1", true)]
#[case::v6_2002_1("2002::1", true)]
#[case::v6_3fff_1("3fff::1", true)]
#[case::v6_3ffe_1("3ffe::1", false)]
#[case::v6_fc00_1("fc00::1", true)]
#[case::v6_fdff_1("fdff::1", true)]
#[case::v6_fe00_1("fe00::1", false)]
#[case::v6_fe80_1("fe80::1", true)]
#[case::v6_febf_1("febf::1", true)]
#[case::v6_fec0_1("fec0::1", false)]
#[case::v6_ff02_1("ff02::1", true)]
#[case::v6_ff00("ff00::", true)]
#[case::v6_2606_4700_1111("2606:4700::1111", false)]
#[case::v6_2a00_1("2a00::1", false)]
#[case::v6_4000_1("4000::1", false)]
#[case::v6_fe80_1_eth0("fe80::1%eth0", true)]
#[case::v6_2001_4860_1_eth0("2001:4860::1%eth0", false)]
#[case::v6_1_2("::1%", true)]
#[case::v6_2001_4860_1_empty_zone("2001:4860::1%", true)]
#[case::bad_empty("", true)]
#[case::bad_abc("abc", true)]
#[case::bad_1_2_3("1.2.3", true)]
#[case::bad_256_1_1_1("256.1.1.1", true)]
#[case::bad_01_2_3_4("01.2.3.4", true)]
#[case::bad_8_8_8_8_eth0("8.8.8.8%eth0", true)]
#[case::bad_8_8_8_8(" 8.8.8.8", true)]
#[case::bad_8_8_8_8_2("8.8.8.8 ", true)]
#[case::bad_1_2_3_4_5("1.2.3.4.5", true)]
#[case::bad_g("::g", true)]
#[case::bad_1_2_3_4_5_6_7_8_9("1:2:3:4:5:6:7:8:9", true)]
#[case::bad_1("[::1]", true)]
fn blocked_addresses_follow_python_313(#[case] address: &str, #[case] blocked: bool) {
    assert_eq!(is_blocked_text(address), blocked);
}

#[rstest]
#[case::relative_sibling("http://a.test/dir/page?x=1", "other", "http://a.test/dir/other")]
#[case::relative_parent_with_dots(
    "http://a.test/dir/page",
    "../up/./x?y=2",
    "http://a.test/up/x?y=2"
)]
#[case::absolute_path("http://a.test/dir/page", "/abs/path", "http://a.test/abs/path")]
#[case::protocol_relative("http://a.test/dir/page", "//b.test/x", "http://b.test/x")]
#[case::query_only("http://a.test/dir/page?x=1", "?y=2", "http://a.test/dir/page?y=2")]
#[case::fragment_only(
    "http://a.test/dir/page?x=1",
    "#frag",
    "http://a.test/dir/page?x=1#frag"
)]
#[case::absolute_normalized(
    "http://a.test/dir/page",
    "https://B.TEST:443/Path with space",
    "https://b.test/Path%20with%20space"
)]
#[case::dots_above_root("http://a.test:8080/dir/", "x/../../..", "http://a.test:8080/")]
#[case::scheme_without_host("http://a.test/dir/page", "http:/relative", "http://a.test/relative")]
#[case::other_scheme("http://a.test/dir/page", "ftp://c.test/f", "ftp://c.test/f")]
#[case::params_kept("http://a.test/dir/page;p=1", "z;q=2", "http://a.test/dir/z;q=2")]
#[case::userinfo_kept_on_relative("http://user:pw@a.test/dir", "/x", "http://user:pw@a.test/x")]
#[case::ipv6_base("http://[::1]:8080/a/b", "c", "http://[::1]:8080/a/c")]
#[case::ipv6_target(
    "http://a.test/a/b",
    "http://[2001:db8::1]/p",
    "http://[2001:db8::1]/p"
)]
#[case::idna_target("http://a.test/a/b", "http://\u{e9}.test/p", "http://xn--9ca.test/p")]
#[case::double_slash_only("http://a.test/a/b", "//", "http://a.test/a/b")]
#[case::mailto_target(
    "http://a.test/a/b",
    "mailto:x@y.test",
    "error: For absolute URLs, path must be empty or begin with '/'"
)]
#[case::base_scheme_and_host_lowercased("HTTP://A.TEST/a/b", "c", "http://a.test/a/c")]
#[case::empty_segments_collapsed("http://a.test/a//b/", "../c//d", "http://a.test/a/c/d")]
#[case::base_without_path("http://a.test", "x", "http://a.test/x")]
#[case::default_port_dropped("http://a.test/a", "http://a.test:80/b", "http://a.test/b")]
#[case::fragment_carried("http://a.test/a#top", "/b", "http://a.test/b#top")]
#[case::fragment_replaced("http://a.test/a#top", "/b#new", "http://a.test/b#new")]
#[case::fragment_carried_to_absolute(
    "http://a.test/a#top",
    "https://c.test/b",
    "https://c.test/b#top"
)]
#[case::invalid_port(
    "http://a.test/a",
    "http://c.test:abc/b",
    "error: Invalid URL in location header: Invalid port: 'abc'."
)]
#[case::invalid_ipv4(
    "http://a.test/a",
    "http://999.1.1.1/b",
    "error: Invalid URL in location header: Invalid IPv4 address: '999.1.1.1'."
)]
#[case::unclosed_bracket(
    "http://a.test/a",
    "http://[::1/b",
    "error: Invalid URL in location header: Invalid port: ':1'."
)]
#[case::control_character(
    "http://a.test/a",
    "/b\u{01}c",
    "error: Invalid URL in location header: Invalid non-printable ASCII character in URL, '\\x01' at position 2.."
)]
#[case::non_ascii_path(
    "http://a.test/a",
    "/\u{e9}?q=\u{fc}#\u{f6}",
    "http://a.test/%C3%A9?q=%C3%BC#%C3%B6"
)]
#[case::percent_escapes_kept(
    "http://a.test/a",
    "/%41%zz?x=%20 y",
    "http://a.test/%41%zz?x=%20%20y"
)]
#[case::backslash_and_brace("http://a.test/a", "/b\\c{d}|e", "http://a.test/b\\c%7Bd%7D|e")]
#[case::three_digit_groups_are_a_host("http://a.test/a", "http://1.2.3/b", "http://1.2.3/b")]
#[case::a_trailing_dot_group_is_a_host("http://a.test/a", "http://1.2.3./b", "http://1.2.3./b")]
#[case::ftp_drops_its_default_port("http://a.test/a", "ftp://c.test:21/f", "ftp://c.test/f")]
#[case::dot_segments_in_a_location("http://a.test/a", "/a/../b", "http://a.test/b")]
#[case::dot_segments_above_the_root("http://a.test/a", "/../a", "http://a.test/a")]
#[case::protocol_relative_double_slash("http://a.test/a", "//b.test//x", "http://b.test//x")]
#[case::userinfo_only_authority("http://a.test/a", "//user@/a/../b", "http://user@/b")]
#[case::port_only_authority("http://a.test/a", "//:80/a/../b", "http:/b")]
#[case::scheme_only_location_normalizes(
    "http://a.test/a",
    "mailto:a/../b",
    "error: For absolute URLs, path must be empty or begin with '/'"
)]
fn redirect_urls_follow_httpx(#[case] request: &str, #[case] target: &str, #[case] expected: &str) {
    let observed = match redirect_url(&HttpxUrl::parse(request).unwrap(), target) {
        Ok(url) => url.href(),
        Err(error) => format!("error: {error}"),
    };
    assert_eq!(observed, expected);
}

#[rstest]
#[case::three_digit_groups_are_a_host("http://1.2.3/b", "http://1.2.3/b")]
#[case::a_trailing_dot_group_is_a_host("http://1.2.3./b", "http://1.2.3./b")]
#[case::ftp_drops_its_default_port("ftp://c.test:21/f", "ftp://c.test/f")]
#[case::dot_segments_dropped("http://a.test/a/../b", "http://a.test/b")]
#[case::dot_segments_above_the_root_dropped("http://a.test/../a", "http://a.test/a")]
#[case::protocol_relative_double_slash_kept("//b.test//x", "//b.test//x")]
#[case::userinfo_alone_is_an_authority("//user@/a/../b", "//user@/b")]
#[case::port_alone_is_an_authority("//:80/a/../b", "//:80/b")]
#[case::scheme_only_url_normalizes_its_path("mailto:a/../b", "mailto:b")]
fn httpx_urls_normalize_like_httpx(#[case] text: &str, #[case] expected: &str) {
    let observed = match HttpxUrl::parse(text) {
        Ok(url) => url.href(),
        Err(error) => format!("error: {error}"),
    };
    assert_eq!(observed, expected);
}

#[test]
fn urls_longer_than_the_httpx_limit_fail() {
    let page = "x".repeat(0x0001_0000 - "http://a.test/".len());
    HttpxUrl::parse(&format!("http://a.test/{page}")).unwrap();
    assert_eq!(
        HttpxUrl::parse(&format!("http://a.test/{page}x")),
        Err(InvalidUrl::TooLong)
    );
}

fn describe_parsed(parsed: &Parsed) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        parsed.scheme, parsed.netloc, parsed.path, parsed.params, parsed.query, parsed.fragment
    )
}

#[rstest]
#[case::digit_only_scheme_is_no_scheme("1:2", "||1:2|||")]
#[case::params_only_for_uses_params_schemes("custom:a;b", "custom||a;b|||")]
#[case::http_splits_the_last_segment_params("http://a.test/a;b", "http|a.test|/a|b||")]
#[case::nfkc_without_separators_is_kept("http://\u{fb01}.test/x", "http|\u{fb01}.test|/x|||")]
#[case::netloc_without_scheme("//host/x", "|host|/x|||")]
#[case::a_scheme_only_url_keeps_its_dot_segments("mailto:a/../b", "mailto||a/../b|||")]
fn urlparse_matches_python_313(#[case] url: &str, #[case] expected: &str) {
    let parsed = urlparse(url, "").unwrap();
    assert_eq!(describe_parsed(&parsed), expected);
}

#[rstest]
#[case::a_path_under_an_empty_netloc("http:/x", "http:///x")]
#[case::a_scheme_with_an_empty_path("mailto:", "mailto:")]
#[case::an_opaque_scheme_keeps_its_slash("mailto:/x", "mailto:/x")]
#[case::four_slashes_stay_after_an_opaque_scheme("mailto:////x", "mailto:////x")]
#[case::a_netloc_without_a_scheme("//host/x", "//host/x")]
#[case::params_are_joined_back_to_the_path("http://a.test/a;b", "http://a.test/a;b")]
#[case::an_opaque_scheme_keeps_its_params("custom:a;b", "custom:a;b")]
fn urlunparse_matches_python_313(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(urlparse(url, "").unwrap().unparse(), expected);
}

#[test]
fn urlparse_rejects_a_netloc_that_nfkc_turns_into_separators() {
    let observed = urlparse("http://a\u{2100}.test/x", "")
        .unwrap_err()
        .to_string();
    assert_eq!(
        observed,
        "netloc 'a\u{2100}.test' contains invalid characters under NFKC normalization"
    );
}

#[rstest]
#[case::a_relative_path_keeps_the_base_directory(
    "http://a.test/a/b",
    "a/b/c",
    "http://a.test/a/a/b/c"
)]
#[case::a_base_without_a_host_keeps_the_leading_slash("http:/a/b", "c", "http:///a/c")]
#[case::an_opaque_scheme_returns_the_reference("mailto:a", "mailto:x/../y", "mailto:x/../y")]
fn urljoin_matches_python_313(#[case] base: &str, #[case] url: &str, #[case] expected: &str) {
    assert_eq!(urljoin(base, url).unwrap(), expected);
}

fn describe_headers(headers: &HeaderMap) -> String {
    format!(
        "auth={} host={} cookie={} keep={}",
        headers.contains_key(AUTHORIZATION),
        headers.contains_key(HOST),
        headers.contains_key(COOKIE),
        headers.contains_key("x-keep"),
    )
}

#[rstest]
#[case::same_origin(
    "http://a.test/a",
    "http://a.test/b",
    "auth=true host=true cookie=false keep=true"
)]
#[case::another_host(
    "http://a.test/a",
    "http://b.test/b",
    "auth=false host=false cookie=false keep=true"
)]
#[case::an_https_upgrade_keeps_authorization(
    "http://a.test/a",
    "https://a.test/b",
    "auth=true host=false cookie=false keep=true"
)]
#[case::an_https_downgrade_drops_authorization(
    "https://a.test/a",
    "http://a.test/b",
    "auth=false host=false cookie=false keep=true"
)]
#[case::another_port_drops_authorization(
    "http://a.test/a",
    "http://a.test:8080/b",
    "auth=false host=false cookie=false keep=true"
)]
#[case::an_https_upgrade_to_another_host_drops_authorization(
    "http://a.test/a",
    "https://b.test/b",
    "auth=false host=false cookie=false keep=true"
)]
#[case::an_https_upgrade_from_another_port_drops_authorization(
    "http://a.test:8080/a",
    "https://a.test/b",
    "auth=false host=false cookie=false keep=true"
)]
fn redirect_headers_follow_httpx(#[case] from: &str, #[case] to: &str, #[case] expected: &str) {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer k"));
    headers.insert(COOKIE, HeaderValue::from_static("s=1"));
    headers.insert(HOST, HeaderValue::from_static("a.test"));
    headers.insert("x-keep", HeaderValue::from_static("v"));
    let next = redirect_headers(
        &headers,
        &HttpxUrl::parse(from).unwrap(),
        &HttpxUrl::parse(to).unwrap(),
    );
    assert_eq!(describe_headers(&next), expected);
}

const INVALID_UTF8: &[u8] = b"\xff";

#[rstest]
#[case::valid_utf8(b"/na\xc3\xafve", None, "/na\u{ef}ve")]
#[case::latin1(b"/na\xefve", None, "/na\u{ef}ve")]
#[case::an_invalid_value_makes_every_value_latin1(
    b"/na\xc3\xafve",
    Some(INVALID_UTF8),
    "/na\u{c3}\u{af}ve"
)]
fn location_headers_decode_like_httpx(
    #[case] value: &[u8],
    #[case] other: Option<&[u8]>,
    #[case] expected: &str,
) {
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, HeaderValue::from_bytes(value).unwrap());
    if let Some(bytes) = other {
        headers.insert("x-other", HeaderValue::from_bytes(bytes).unwrap());
    }
    assert_eq!(location(&headers).as_deref(), Some(expected));
}

#[test]
fn repeated_location_headers_join_with_a_comma() {
    let mut headers = HeaderMap::new();
    headers.append(LOCATION, HeaderValue::from_static("/x"));
    headers.append(LOCATION, HeaderValue::from_static("/y"));
    assert_eq!(location(&headers).as_deref(), Some("/x, /y"));
}

#[rstest]
fn replays_the_user_url_corpus(#[files("fuzz/corpus/user_url/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    let (base, target) = text.split_once('\n').unwrap_or((&text, ""));
    let _blocked = is_blocked_text(base);
    if let Ok(parsed) = urlparse(base, "") {
        let _host = (parsed.hostname(), parsed.port());
        let _unparsed = parsed.unparse();
    }
    let _joined = urljoin(base, target);
    if let Ok(request) = HttpxUrl::parse(base) {
        let _redirect = redirect_url(&request, target);
    }
}

// A server that answers from a table of "path status|header|header" lines and notes each request it gets
struct Servers {
    first: SocketAddr,
    second: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    tasks: JoinSet<()>,
}

fn answer(spec: &str) -> (String, Vec<u8>) {
    let (path, rest) = spec.split_once(' ').unwrap();
    let head = rest.split('|').collect::<Vec<_>>().join("\r\n");
    (
        path.to_owned(),
        format!("HTTP/1.1 {head}\r\ncontent-length: 0\r\n\r\n").into_bytes(),
    )
}

fn describe(head: &str) -> (String, String) {
    let lowered = head.to_lowercase();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default();
    let host = lowered
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or_default();
    let auth = if lowered.contains("\r\nauthorization: ") {
        " auth"
    } else {
        ""
    };
    let cookie = if lowered.contains("\r\ncookie: ") {
        " cookie"
    } else {
        ""
    };
    (path.to_owned(), format!("{path} @{host}{auth}{cookie}"))
}

async fn converse(
    mut stream: TcpStream,
    routes: Arc<BTreeMap<String, Vec<u8>>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk).await {
                Ok(count) if count > 0 => buffer.extend_from_slice(chunk.get(..count).unwrap()),
                _ => return,
            }
        }
        let end = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            .saturating_add(4);
        let head = String::from_utf8(buffer.drain(..end).collect()).unwrap();
        let (path, line) = describe(&head);
        requests.lock().unwrap().push(line);
        let reply = routes
            .get(path.split('?').next().unwrap_or_default())
            .cloned()
            .unwrap_or_else(|| b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_vec());
        if stream.write_all(&reply).await.is_err() {
            return;
        }
    }
}

async fn accept(
    listener: TcpListener,
    routes: Arc<BTreeMap<String, Vec<u8>>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let mut open = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        open.spawn(converse(stream, Arc::clone(&routes), Arc::clone(&requests)));
    }
}

impl Servers {
    async fn start(first_routes: &[&str], second_routes: &[&str]) -> Self {
        let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut servers = Self {
            first: first.local_addr().unwrap(),
            second: second.local_addr().unwrap(),
            requests: Arc::default(),
            tasks: JoinSet::new(),
        };
        for (listener, specs) in [(first, first_routes), (second, second_routes)] {
            let routes = specs
                .iter()
                .map(|spec| answer(&servers.fill(spec)))
                .collect();
            let requests = Arc::clone(&servers.requests);
            servers
                .tasks
                .spawn(accept(listener, Arc::new(routes), requests));
        }
        servers
    }

    // <app> is a host name the fake resolver sends to the first server, <first> and <second> are literal addresses
    fn fill(&self, text: &str) -> String {
        text.replace("<app>", &format!("app.test:{}", self.first.port()))
            .replace("<first>", &self.first.to_string())
            .replace("<second>", &self.second.to_string())
            .replace("<port>", &self.first.port().to_string())
    }

    fn render(&self, text: &str) -> String {
        text.replace(&format!("app.test:{}", self.first.port()), "<app>")
            .replace(&self.first.to_string(), "<first>")
            .replace(&self.second.to_string(), "<second>")
    }

    // Consecutive repeats of one request collapse to "request xN"
    fn seen(&self) -> Vec<String> {
        let mut lines: Vec<(String, usize)> = Vec::new();
        for raw in self.requests.lock().unwrap().iter() {
            let line = self.render(raw);
            match lines.last_mut() {
                Some((previous, count)) if *previous == line => *count = count.saturating_add(1),
                _ => lines.push((line, 1)),
            }
        }
        lines
            .into_iter()
            .map(|(line, count)| {
                if count > 1 {
                    format!("{line} x{count}")
                } else {
                    line
                }
            })
            .collect()
    }
}

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

struct Run<'input> {
    validation: bool,
    start: &'input str,
    first: &'input [&'input str],
    second: &'input [&'input str],
}

// The caller's Authorization and Cookie headers go with every request
async fn fetch_status(run: Run<'_>) -> (String, Vec<String>) {
    let servers = Servers::start(run.first, run.second).await;
    let allowed: Vec<String> = if run.validation {
        vec!["127.0.0.1".to_owned(), "app.test".to_owned()]
    } else {
        Vec::new()
    };
    let validator = UrlValidator::new(FakeResolver, &allowed, &Verification::Enabled);
    let fetcher = UserUrlFetcher {
        validator,
        validation: run.validation,
    };
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer k"));
    headers.insert(COOKIE, HeaderValue::from_static("s=1"));
    let outcome = match fetcher
        .fetch(&client(), &servers.fill(run.start), &headers)
        .await
    {
        Ok(streamed) => streamed.response.status().as_u16().to_string(),
        Err(error) => error.to_string().replace(HINT, "<hint>"),
    };
    (outcome, servers.seen())
}

#[rstest]
#[case::chain_resolves_relative_and_absolute_locations(
    "http://<app>/",
    &[
        "/ 302 Found|location: /two",
        "/two 301 Moved Permanently|location: http://<app>/three?x=1",
        "/three 200 OK",
    ],
    &[],
    "200",
    &["/ @<app> auth cookie", "/two @<app> auth cookie", "/three?x=1 @<app> auth cookie"],
)]
#[case::headers_follow_a_redirect_to_another_origin(
    "http://<app>/",
    &["/ 307 Temporary Redirect|location: http://<second>/landing"],
    &["/landing 200 OK"],
    "200",
    &["/ @<app> auth cookie", "/landing @<second> auth cookie"],
)]
#[case::start_url_on_loopback_is_blocked(
    "http://127.0.0.2:<port>/",
    &["/ 200 OK"],
    &[],
    "URL targets a blocked address (127.0.0.2)<hint>",
    &[],
)]
#[case::redirect_to_loopback_is_blocked(
    "http://<app>/",
    &["/ 302 Found|location: http://127.0.0.2:<port>/secret"],
    &[],
    "URL targets a blocked address (127.0.0.2)<hint>",
    &["/ @<app> auth cookie"],
)]
#[case::redirect_to_private_network_is_blocked(
    "http://<app>/",
    &["/ 302 Found|location: http://10.0.0.1/x"],
    &[],
    "URL targets a blocked address (10.0.0.1)<hint>",
    &["/ @<app> auth cookie"],
)]
#[case::redirect_to_another_scheme_is_refused(
    "http://<app>/",
    &["/ 302 Found|location: ftp://app.test/x"],
    &[],
    "URL scheme 'ftp' is not allowed",
    &["/ @<app> auth cookie"],
)]
#[case::empty_location_is_refused(
    "http://<app>/",
    &["/ 302 Found|location: "],
    &[],
    "Redirect response has no Location header",
    &["/ @<app> auth cookie"],
)]
#[case::redirect_without_location_is_refused(
    "http://<app>/",
    &["/ 302 Found"],
    &[],
    "Redirect response has no Location header",
    &["/ @<app> auth cookie"],
)]
#[case::a_300_status_is_followed(
    "http://<app>/",
    &["/ 300 Multiple Choices|location: /two", "/two 200 OK"],
    &[],
    "200",
    &["/ @<app> auth cookie", "/two @<app> auth cookie"],
)]
#[case::a_utf8_location_is_quoted_before_the_next_hop(
    "http://<app>/",
    &["/ 302 Found|location: /na\u{ef}ve", "/na%C3%AFve 200 OK"],
    &[],
    "200",
    &["/ @<app> auth cookie", "/na%C3%AFve @<app> auth cookie"],
)]
#[case::redirect_loop_stops_at_ten_requests(
    "http://<app>/",
    &["/ 302 Found|location: /"],
    &[],
    "Too many redirects",
    &["/ @<app> auth cookie x10"],
)]
#[tokio::test]
async fn redirect_hops_are_validated(
    #[case] start: &str,
    #[case] first: &[&str],
    #[case] second: &[&str],
    #[case] expected: &str,
    #[case] requests: &[&str],
) {
    let run = Run {
        validation: true,
        start,
        first,
        second,
    };
    let (outcome, seen) = fetch_status(run).await;
    assert_eq!(
        (outcome, seen),
        (
            expected.to_owned(),
            requests
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>()
        )
    );
}

#[rstest]
#[case::chain_follows_relative_and_absolute_locations(
    "http://<first>/",
    &[
        "/ 302 Found|location: /two",
        "/two 308 Permanent Redirect|location: http://<first>/three",
        "/three 200 OK",
    ],
    &[],
    "200",
    &["/ @<first> auth cookie", "/two @<first> auth", "/three @<first> auth"],
)]
#[case::cross_origin_redirect_drops_authorization(
    "http://<first>/",
    &["/ 302 Found|location: http://<second>/landing"],
    &["/landing 200 OK"],
    "200",
    &["/ @<first> auth cookie", "/landing @<second>"],
)]
#[case::redirect_loop_stops_after_twenty_one_requests(
    "http://<first>/",
    &["/ 302 Found|location: /"],
    &[],
    "Exceeded maximum allowed redirects.",
    &["/ @<first> auth cookie", "/ @<first> auth x20"],
)]
#[case::redirect_without_location_is_returned(
    "http://<first>/",
    &["/ 302 Found"],
    &[],
    "302",
    &["/ @<first> auth cookie"],
)]
#[case::a_non_redirect_status_returns_even_with_a_location(
    "http://<first>/",
    &["/ 200 OK|location: /two", "/two 200 OK"],
    &[],
    "200",
    &["/ @<first> auth cookie"],
)]
#[case::an_invalid_location_names_the_header(
    "http://<first>/",
    &["/ 302 Found|location: http://[::1/b"],
    &[],
    "Invalid URL in location header: Invalid port: ':1'.",
    &["/ @<first> auth cookie"],
)]
#[tokio::test]
async fn unvalidated_fetch_follows_redirects(
    #[case] start: &str,
    #[case] first: &[&str],
    #[case] second: &[&str],
    #[case] expected: &str,
    #[case] requests: &[&str],
) {
    let run = Run {
        validation: false,
        start,
        first,
        second,
    };
    let (outcome, seen) = fetch_status(run).await;
    assert_eq!(
        (outcome, seen),
        (
            expected.to_owned(),
            requests
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>()
        )
    );
}
