#![cfg(test)]

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use ipnet::IpNet;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

use crate::auth_identity::records::NetworkContext;
use crate::trusted_network::cidr::{
    CidrRanges, ip_in_networks, normalize_cidr_ranges, parse_normalized, parse_trusted_proxy_ranges,
};
use crate::trusted_network::decode::{
    DecodeError, MAX_NETWORK_HEADER_BYTES, MAX_NETWORK_HEADER_DEPTH, TypedValue,
    decode_network_headers, encode_network_headers,
};
use crate::trusted_network::internal::{
    DEFAULT_INTERNAL_CIDRS, default_internal_networks, is_internal_ip, is_trusted_proxy,
    parse_internal_networks, parse_trusted_proxy_networks,
};
use crate::trusted_network::proxy_utils::{
    MCP_TRUSTED_PROXY_RANGES_KEY, TRUSTED_PROXY_RANGES_KEY, TrustedProxyRequirementError,
    get_trusted_proxy_cidrs, principal_proxy_config, require_trusted_proxy_request,
};
use crate::trusted_network::request::{
    NetworkRequest, TrustedHopCount, extract_client_ip_from_xff_hops, get_mcp_client_ip,
    is_request_from_trusted_proxy, is_request_https, resolve_client_ip, resolve_network_context,
    resolve_num_trusted_hops,
};
use crate::trusted_network::settings::{TrustedNetworkSettings, TrustedProxyConfig};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

struct RecordedLog(String);

impl Visit for RecordedLog {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = RecordedLog(String::new());
        event.record(&mut recorded);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{} {}", event.metadata().level(), recorded.0));
    }
}

fn list(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

fn networks(texts: &[&str]) -> Vec<IpNet> {
    parse_normalized(&list(texts), "trusted_proxy_cidrs")
}

fn rendered(networks: &[IpNet]) -> Vec<String> {
    networks.iter().map(ToString::to_string).collect()
}

fn defaults() -> TrustedNetworkSettings {
    TrustedNetworkSettings::default()
}

fn proxy_config(texts: &[&str]) -> TrustedProxyConfig {
    TrustedProxyConfig {
        use_forwarded_for: !texts.is_empty(),
        trusted_proxy_cidrs: list(texts),
    }
}

fn request<'request>(peer: Option<&'request str>) -> NetworkRequest<'request> {
    NetworkRequest {
        peer,
        x_forwarded_for: None,
        x_forwarded_proto: None,
        host: None,
        scheme: "",
    }
}

fn forwarded<'request>(
    peer: Option<&'request str>,
    x_forwarded_for: &'request str,
) -> NetworkRequest<'request> {
    NetworkRequest {
        peer,
        x_forwarded_for: Some(x_forwarded_for),
        x_forwarded_proto: None,
        host: None,
        scheme: "",
    }
}

fn with_proto<'request>(
    peer: Option<&'request str>,
    x_forwarded_proto: &'request str,
    scheme: &'request str,
) -> NetworkRequest<'request> {
    NetworkRequest {
        peer,
        x_forwarded_for: None,
        x_forwarded_proto: Some(x_forwarded_proto),
        host: None,
        scheme,
    }
}

fn mcp_settings(texts: &[&str]) -> TrustedNetworkSettings {
    let mut settings = defaults();
    settings.use_x_forwarded_for = true;
    settings.mcp_trusted_proxy_ranges = Some(list(texts));
    settings
}

#[test]
fn normalizes_text_and_list_ranges() {
    assert!(normalize_cidr_ranges(None).is_empty());
    assert_eq!(
        normalize_cidr_ranges(Some(&CidrRanges::Text(
            " 10.0.0.0/8 , 192.168.0.0/16 ,, ".to_owned()
        ))),
        list(&["10.0.0.0/8", "192.168.0.0/16"])
    );
    assert_eq!(
        normalize_cidr_ranges(Some(&CidrRanges::List(list(&[" 10.0.0.0/8 ", " ", ""])))),
        list(&["10.0.0.0/8"])
    );
}

#[test]
fn parses_networks_like_ipaddress() {
    let parsed = parse_trusted_proxy_ranges(
        Some(&CidrRanges::Text(
            "10.1.2.3/8, 10.0.0.0/255.0.0.0, 192.168.1.1, 2001:db8::/32".to_owned(),
        )),
        "trusted_proxy_ranges",
    );
    assert_eq!(
        rendered(&parsed),
        list(&[
            "10.0.0.0/8",
            "10.0.0.0/8",
            "192.168.1.1/32",
            "2001:db8::/32"
        ])
    );
    assert!(parse_trusted_proxy_ranges(None, "trusted_proxy_ranges").is_empty());
    assert!(parse_trusted_proxy_ranges(Some(&CidrRanges::List(Vec::new())), "x").is_empty());
}

#[test]
fn invalid_cidr_warning() {
    let logs = Logs::default();
    let subscriber = Registry::default().with(logs.clone());
    let guard = subscriber::set_default(subscriber);
    let parsed = parse_trusted_proxy_ranges(
        Some(&CidrRanges::List(list(&["not-a-cidr", " 10.0.0.0/8 ", ""]))),
        "trusted_proxy_ranges",
    );
    drop(guard);
    let events = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(rendered(&parsed), list(&["10.0.0.0/8"]));
    assert_eq!(
        events,
        vec!["WARN Invalid CIDR in trusted_proxy_ranges: not-a-cidr, skipping".to_owned()]
    );
    // an internal-range list with no usable entry falls back to the documented defaults
    let internal = parse_internal_networks(Some(&list(&["not-a-range", ""])));
    assert_eq!(rendered(&internal), rendered(&default_internal_networks()));
}

#[test]
fn internal_networks_fall_back_to_defaults() {
    assert_eq!(
        rendered(&default_internal_networks()),
        list(&DEFAULT_INTERNAL_CIDRS)
    );
    assert_eq!(
        rendered(&parse_internal_networks(None)),
        list(&DEFAULT_INTERNAL_CIDRS)
    );
    assert_eq!(
        rendered(&parse_internal_networks(Some(&[]))),
        list(&DEFAULT_INTERNAL_CIDRS)
    );
    // a parsable entry keeps the configured list as it is, without adding the defaults
    let custom = parse_internal_networks(Some(&list(&["203.0.113.0/24", "nope"])));
    assert_eq!(rendered(&custom), list(&["203.0.113.0/24"]));
}

#[test]
fn trusted_proxy_networks_keep_no_default() {
    assert!(parse_trusted_proxy_networks(None).is_empty());
    assert!(parse_trusted_proxy_networks(Some(&[])).is_empty());
    let parsed = parse_trusted_proxy_networks(Some(&list(&["10.0.0.0/8", "nope"])));
    assert_eq!(rendered(&parsed), list(&["10.0.0.0/8"]));
    assert!(parse_trusted_proxy_networks(Some(&list(&["nope"]))).is_empty());
}

#[test]
fn matches_trusted_proxy_peers() {
    let trusted = networks(&["10.0.0.0/8", "2001:db8::/32"]);
    assert!(is_trusted_proxy(Some("10.1.2.3"), &trusted));
    assert!(is_trusted_proxy(Some(" 10.1.2.3 "), &trusted));
    assert!(is_trusted_proxy(Some("2001:db8::7"), &trusted));
    assert!(!is_trusted_proxy(Some("203.0.113.9"), &trusted));
    assert!(!is_trusted_proxy(Some("nope"), &trusted));
    assert!(!is_trusted_proxy(Some(""), &trusted));
    assert!(!is_trusted_proxy(None, &trusted));
    assert!(!is_trusted_proxy(Some("10.1.2.3"), &[]));
}

#[test]
fn ipv4_mapped_ipv6() {
    let private = networks(&["10.0.0.0/8", "::1/128"]);
    assert!(ip_in_networks(Some("::ffff:10.1.2.3"), &private));
    assert!(ip_in_networks(Some("10.1.2.3"), &private));
    assert!(!ip_in_networks(Some("::ffff:192.168.1.1"), &private));
    assert!(!ip_in_networks(Some("nope"), &private));
    // is_trusted_proxy compares the literal peer, so a mapped peer never matches an IPv4 range
    assert!(!is_trusted_proxy(
        Some("::ffff:10.1.2.3"),
        &networks(&["10.0.0.0/8"])
    ));
    assert!(!is_trusted_proxy(
        Some("10.1.2.3"),
        &networks(&["::ffff:10.0.0.0/104"])
    ));
    // ...while the client-IP resolver normalizes before its CIDR and forwarded-hop checks
    let mapped = forwarded(Some("::ffff:10.0.0.5"), "::ffff:203.0.113.9");
    assert_eq!(
        resolve_client_ip(&mapped, &proxy_config(&["10.0.0.0/8"])),
        (Some("::ffff:203.0.113.9".to_owned()), true)
    );
    let mapped_hop = forwarded(Some("10.0.0.5"), "203.0.113.9, ::ffff:10.0.0.8");
    assert_eq!(
        resolve_client_ip(&mapped_hop, &proxy_config(&["10.0.0.0/8"])),
        (Some("203.0.113.9".to_owned()), true)
    );
}

#[test]
fn untrusted_spoofed_xff() {
    let config = proxy_config(&["10.0.0.0/8"]);
    let spoofed = forwarded(Some("203.0.113.9"), "10.0.0.7, 198.51.100.4");
    assert_eq!(
        resolve_client_ip(&spoofed, &config),
        (Some("203.0.113.9".to_owned()), false)
    );
    let context = resolve_network_context(&spoofed, &config);
    assert_eq!(context.client_ip.as_deref(), Some("203.0.113.9"));
    assert!(!context.via_trusted_proxy);
    // a peer outside the configured ranges never gets its header believed
    let private_peer = forwarded(Some("192.168.1.4"), "10.0.0.7");
    assert_eq!(
        resolve_client_ip(&private_peer, &config),
        (Some("192.168.1.4".to_owned()), false)
    );
    // the toggle alone disables the walk, even for a trusted peer
    let disabled = TrustedProxyConfig {
        use_forwarded_for: false,
        trusted_proxy_cidrs: list(&["10.0.0.0/8"]),
    };
    let trusted_peer = forwarded(Some("10.0.0.5"), "10.0.0.7");
    assert_eq!(
        resolve_client_ip(&trusted_peer, &disabled),
        (Some("10.0.0.5".to_owned()), false)
    );
    // the MCP client reader falls back to the direct peer instead of the spoofed header
    assert_eq!(
        get_mcp_client_ip(&spoofed, &mcp_settings(&["10.0.0.0/8"])),
        Some("203.0.113.9".to_owned())
    );
}

#[test]
fn trusted_rightmost_hop() {
    let config = proxy_config(&["10.0.0.0/8", "198.51.100.0/24"]);
    let chain = forwarded(
        Some("10.0.0.5"),
        "spoof.example, 203.0.113.7, 198.51.100.9, 10.0.0.8",
    );
    assert_eq!(
        resolve_client_ip(&chain, &config),
        (Some("203.0.113.7".to_owned()), true)
    );
    // every hop is a trusted proxy, so the direct peer stays the client
    let all_proxies = forwarded(Some("10.0.0.5"), "10.0.0.8, 198.51.100.9");
    assert_eq!(
        resolve_client_ip(&all_proxies, &config),
        (Some("10.0.0.5".to_owned()), true)
    );
    // an empty header still records that the X-Forwarded-For was trusted
    let empty = forwarded(Some("10.0.0.5"), "  ,  ");
    assert_eq!(
        resolve_client_ip(&empty, &config),
        (Some("10.0.0.5".to_owned()), true)
    );
    // the whole header is the MCP answer without a hop limit
    assert_eq!(
        get_mcp_client_ip(&chain, &mcp_settings(&["10.0.0.0/8"])),
        Some("spoof.example, 203.0.113.7, 198.51.100.9, 10.0.0.8".to_owned())
    );
}

#[test]
fn missing_client() {
    let config = proxy_config(&["10.0.0.0/8"]);
    assert_eq!(resolve_client_ip(&request(None), &config), (None, false));
    assert_eq!(
        resolve_network_context(&request(None), &config),
        NetworkContext {
            client_ip: None,
            host: None,
            via_trusted_proxy: false,
        }
    );
    // no peer and no header: the legacy answer is "" so access control fails closed
    assert_eq!(
        get_mcp_client_ip(&request(None), &defaults()),
        Some(String::new())
    );
    // a header without trusted ranges fails closed instead of trusting the direct peer
    assert_eq!(
        get_mcp_client_ip(
            &forwarded(Some("8.8.8.8"), "203.0.113.7"),
            &mcp_settings(&[])
        ),
        Some(String::new())
    );
    // configured ranges with no peer return the missing direct address, as LiteLLM's direct_ip does
    assert_eq!(
        get_mcp_client_ip(
            &forwarded(None, "203.0.113.7"),
            &mcp_settings(&["10.0.0.0/8"])
        ),
        None
    );
}

#[test]
fn mcp_hop_limit() {
    let mut settings = mcp_settings(&["10.0.0.0/8"]);
    let proxy = Some("10.0.0.5");
    assert_eq!(
        get_mcp_client_ip(&forwarded(proxy, "spoof, 203.0.113.7, 10.0.0.8"), &settings),
        Some("spoof, 203.0.113.7, 10.0.0.8".to_owned())
    );
    settings.mcp_xff_num_trusted_hops = Some(2);
    assert_eq!(
        get_mcp_client_ip(&forwarded(proxy, "spoof, 203.0.113.7, 10.0.0.8"), &settings),
        Some("203.0.113.7".to_owned())
    );
    settings.mcp_xff_num_trusted_hops = Some(4);
    assert_eq!(
        get_mcp_client_ip(&forwarded(proxy, "spoof, 203.0.113.7, 10.0.0.8"), &settings),
        Some(String::new())
    );
    settings.mcp_xff_num_trusted_hops = Some(1);
    assert_eq!(
        get_mcp_client_ip(&forwarded(proxy, "203.0.113.7, not-an-ip"), &settings),
        Some(String::new())
    );
    // a present value below one fails closed instead of reverting to the legacy header
    settings.mcp_xff_num_trusted_hops = Some(0);
    assert_eq!(
        get_mcp_client_ip(&forwarded(proxy, "203.0.113.7"), &settings),
        Some(String::new())
    );
    settings.mcp_xff_num_trusted_hops = Some(-2);
    assert_eq!(
        get_mcp_client_ip(&forwarded(proxy, "203.0.113.7"), &settings),
        Some(String::new())
    );
}

#[test]
fn hop_count_setting_branches() {
    assert_eq!(resolve_num_trusted_hops(None), TrustedHopCount::Unset);
    assert_eq!(resolve_num_trusted_hops(Some(1)), TrustedHopCount::Count(1));
    assert_eq!(resolve_num_trusted_hops(Some(7)), TrustedHopCount::Count(7));
    assert_eq!(resolve_num_trusted_hops(Some(0)), TrustedHopCount::Invalid);
    assert_eq!(resolve_num_trusted_hops(Some(-2)), TrustedHopCount::Invalid);
}

#[test]
fn extract_hops_from_the_right() {
    assert_eq!(
        extract_client_ip_from_xff_hops(" 1.2.3.4 , 5.6.7.8 ", 1),
        Some("5.6.7.8".to_owned())
    );
    assert_eq!(
        extract_client_ip_from_xff_hops("1.2.3.4, 5.6.7.8", 2),
        Some("1.2.3.4".to_owned())
    );
    assert_eq!(
        extract_client_ip_from_xff_hops("1.2.3.4, , 5.6.7.8", 2),
        Some("1.2.3.4".to_owned())
    );
    assert_eq!(extract_client_ip_from_xff_hops("1.2.3.4", 2), None);
    assert_eq!(extract_client_ip_from_xff_hops("1.2.3.4", 0), None);
    assert_eq!(extract_client_ip_from_xff_hops("1.2.3.4, nope", 1), None);
    assert_eq!(extract_client_ip_from_xff_hops("", 1), None);
}

#[test]
fn request_from_trusted_proxy_gating() {
    let mut settings = defaults();
    let peer_inside = request(Some("10.0.0.5"));
    assert!(!is_request_from_trusted_proxy(&peer_inside, &settings));
    settings.use_x_forwarded_for = true;
    assert!(!is_request_from_trusted_proxy(&peer_inside, &settings));
    settings.mcp_trusted_proxy_ranges = Some(Vec::new());
    assert!(!is_request_from_trusted_proxy(&peer_inside, &settings));
    settings.mcp_trusted_proxy_ranges = Some(list(&["10.0.0.0/8"]));
    assert!(is_request_from_trusted_proxy(&peer_inside, &settings));
    // the toggle still decides on its own, even with the ranges configured
    settings.use_x_forwarded_for = false;
    assert!(!is_request_from_trusted_proxy(&peer_inside, &settings));
    settings.use_x_forwarded_for = true;
    assert!(!is_request_from_trusted_proxy(
        &request(Some("203.0.113.9")),
        &settings
    ));
    assert!(!is_request_from_trusted_proxy(&request(None), &settings));
    settings.mcp_trusted_proxy_ranges = Some(list(&["nope"]));
    assert!(!is_request_from_trusted_proxy(&peer_inside, &settings));
}

#[test]
fn logs_only_the_ignored_header_and_unconfigured_ranges() {
    let logs = Logs::default();
    let subscriber = Registry::default().with(logs.clone());
    let guard = subscriber::set_default(subscriber);
    // a header while the toggle is off: the direct peer answers and the drop is logged
    assert_eq!(
        get_mcp_client_ip(&forwarded(Some("10.0.0.5"), "203.0.113.7"), &defaults()),
        Some("10.0.0.5".to_owned())
    );
    let trusted = mcp_settings(&["10.0.0.0/8"]);
    // trusted with configured ranges: the header is used and nothing is logged
    assert_eq!(
        get_mcp_client_ip(&forwarded(Some("10.0.0.5"), "203.0.113.7"), &trusted),
        Some("203.0.113.7".to_owned())
    );
    // an empty range list is as untrusted as a missing one, and says so
    assert!(!is_request_from_trusted_proxy(
        &request(Some("10.0.0.5")),
        &mcp_settings(&[])
    ));
    assert!(is_request_from_trusted_proxy(
        &request(Some("10.0.0.5")),
        &trusted
    ));
    drop(guard);
    let events = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        events,
        vec![
            "DEBUG received an X-Forwarded-For header with use_x_forwarded_for disabled; the direct peer is used for MCP access control".to_owned(),
            "DEBUG mcp_trusted_proxy_ranges is not configured, so X-Forwarded-* headers are not trusted".to_owned(),
        ]
    );
}

#[test]
fn https_forwarded_trust() {
    let mut settings = defaults();
    assert!(!is_request_https(&request(None), &settings));
    let mut secure = request(None);
    secure.scheme = "https";
    assert!(is_request_https(&secure, &settings));
    // the literal scheme compare is case-sensitive
    let mut upper = request(None);
    upper.scheme = "HTTPS";
    assert!(!is_request_https(&upper, &settings));

    // PROXY_BASE_URL decides first, and an empty value counts as unset
    settings.proxy_base_url = Some(" https://gateway.example/ ".to_owned());
    assert!(is_request_https(
        &with_proto(Some("10.0.0.5"), "http", "http"),
        &settings
    ));
    settings.proxy_base_url = Some("http://gateway.example".to_owned());
    assert!(!is_request_https(&secure, &settings));
    settings.proxy_base_url = Some("   ".to_owned());
    assert!(is_request_https(&secure, &settings));

    // a trusted proxy's X-Forwarded-Proto decides, in any case and by its first entry only
    let mut configured = mcp_settings(&["10.0.0.0/8"]);
    configured.proxy_base_url = None;
    assert!(is_request_https(
        &with_proto(Some("10.0.0.5"), "HTTPS, http", "http"),
        &configured
    ));
    assert!(!is_request_https(
        &with_proto(Some("10.0.0.5"), "http", "https"),
        &configured
    ));
    // an untrusted peer cannot strip Secure with the header
    assert!(is_request_https(
        &with_proto(Some("203.0.113.9"), "http", "https"),
        &configured
    ));
    // an empty forwarded protocol falls through to the literal scheme
    assert!(is_request_https(
        &with_proto(Some("10.0.0.5"), "", "https"),
        &configured
    ));
}

#[test]
fn principal_config_trusts_only_configured_ranges() {
    assert!(get_trusted_proxy_cidrs(&defaults()).is_empty());
    let mut configured = defaults();
    configured.trusted_proxy_ranges = Some(CidrRanges::Text(
        " 10.0.0.0/8 , ,192.168.0.0/16 ".to_owned(),
    ));
    assert_eq!(
        get_trusted_proxy_cidrs(&configured),
        list(&["10.0.0.0/8", "192.168.0.0/16"])
    );
    let config = principal_proxy_config(&configured);
    assert!(config.use_forwarded_for);
    assert_eq!(
        config.trusted_proxy_cidrs,
        list(&["10.0.0.0/8", "192.168.0.0/16"])
    );
    configured.trusted_proxy_ranges = Some(CidrRanges::Text(" , ".to_owned()));
    let empty = principal_proxy_config(&configured);
    assert!(!empty.use_forwarded_for);
    assert!(empty.trusted_proxy_cidrs.is_empty());
}

#[test]
fn requires_a_trusted_direct_peer() {
    let settings = defaults();
    let error = require_trusted_proxy_request(
        &request(Some("10.0.0.5")),
        &settings,
        "Custom UI SSO",
        TRUSTED_PROXY_RANGES_KEY,
    )
    .unwrap_err();
    assert_eq!(
        error,
        TrustedProxyRequirementError::NotConfigured {
            feature: "Custom UI SSO".to_owned(),
            setting: TRUSTED_PROXY_RANGES_KEY.to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "Custom UI SSO requires general_settings.trusted_proxy_ranges before trusting identity headers from an upstream proxy."
    );

    let mut configured = defaults();
    configured.trusted_proxy_ranges = Some(CidrRanges::List(list(&["10.0.0.0/8"])));
    assert!(
        require_trusted_proxy_request(
            &request(Some("10.0.0.5")),
            &configured,
            "Custom UI SSO",
            TRUSTED_PROXY_RANGES_KEY,
        )
        .is_ok()
    );
    let error = require_trusted_proxy_request(
        &request(Some("203.0.113.9")),
        &configured,
        "Custom UI SSO",
        TRUSTED_PROXY_RANGES_KEY,
    )
    .unwrap_err();
    assert_eq!(
        error,
        TrustedProxyRequirementError::Untrusted {
            feature: "Custom UI SSO".to_owned(),
            direct: "203.0.113.9".to_owned(),
        }
    );
    // a missing peer is untrusted, never trusted
    let error = require_trusted_proxy_request(
        &request(None),
        &configured,
        "Custom UI SSO",
        TRUSTED_PROXY_RANGES_KEY,
    )
    .unwrap_err();
    assert_eq!(
        error,
        TrustedProxyRequirementError::Untrusted {
            feature: "Custom UI SSO".to_owned(),
            direct: "None".to_owned(),
        }
    );
    // only ranges that parse count as configured
    configured.trusted_proxy_ranges = Some(CidrRanges::List(list(&["nope"])));
    assert!(matches!(
        require_trusted_proxy_request(
            &request(Some("10.0.0.5")),
            &configured,
            "Custom UI SSO",
            TRUSTED_PROXY_RANGES_KEY,
        ),
        Err(TrustedProxyRequirementError::NotConfigured { .. })
    ));
    // the MCP setting name reads the MCP ranges
    let mut mcp = defaults();
    mcp.mcp_trusted_proxy_ranges = Some(list(&["10.0.0.0/8"]));
    assert!(
        require_trusted_proxy_request(
            &request(Some("10.0.0.5")),
            &mcp,
            "OAuth2 proxy auth",
            MCP_TRUSTED_PROXY_RANGES_KEY,
        )
        .is_ok()
    );
}

#[test]
fn is_internal_ip_reads_leftmost_hop() {
    assert!(!is_internal_ip(None, None));
    assert!(!is_internal_ip(Some(""), None));
    assert!(is_internal_ip(Some("10.1.2.3"), None));
    assert!(is_internal_ip(Some("10.1.2.3, 8.8.8.8"), None));
    assert!(!is_internal_ip(Some("8.8.8.8, 10.1.2.3"), None));
    assert!(!is_internal_ip(Some("nope"), None));
    let custom = networks(&["203.0.113.0/24"]);
    assert!(is_internal_ip(Some("203.0.113.7"), Some(&custom)));
    assert!(!is_internal_ip(Some("10.1.2.3"), Some(&custom)));
    // an empty custom list falls back to the defaults instead of trusting nobody
    assert!(is_internal_ip(Some("10.1.2.3"), Some(&[])));
}

#[test]
fn resolves_context_fields() {
    let incoming = NetworkRequest {
        peer: Some("10.0.0.5"),
        x_forwarded_for: Some("203.0.113.7"),
        x_forwarded_proto: None,
        host: Some("proxy.example"),
        scheme: "http",
    };
    assert_eq!(
        resolve_network_context(&incoming, &proxy_config(&["10.0.0.0/8"])),
        NetworkContext {
            client_ip: Some("203.0.113.7".to_owned()),
            host: Some("proxy.example".to_owned()),
            via_trusted_proxy: true,
        }
    );
}

#[test]
fn decodes_and_encodes_the_header_document() {
    let value = TypedValue {
        x_forwarded_for: Some("203.0.113.9, 10.0.0.4".to_owned()),
        x_forwarded_proto: Some("https".to_owned()),
        host: Some("proxy.example".to_owned()),
        trusted_proxy_ranges: Some(CidrRanges::List(list(&["10.0.0.0/8"]))),
        mcp_trusted_proxy_ranges: Some(list(&["10.0.0.0/8"])),
    };
    let bytes = encode_network_headers(&value).unwrap();
    assert_eq!(decode_network_headers(&bytes).unwrap(), value);
    assert_eq!(
        decode_network_headers(b"{}").unwrap(),
        TypedValue::default()
    );
    assert_eq!(
        decode_network_headers(br#"{"host":null}"#).unwrap().host,
        None
    );
    assert_eq!(
        encode_network_headers(&TypedValue::default())
            .unwrap()
            .as_slice(),
        b"{}".as_slice()
    );
    let text = decode_network_headers(br#"{"trusted_proxy_ranges":"10.0.0.0/8, 192.168.0.0/16"}"#)
        .unwrap();
    assert_eq!(
        text.trusted_proxy_ranges,
        Some(CidrRanges::Text("10.0.0.0/8, 192.168.0.0/16".to_owned()))
    );
}

#[test]
fn decoder_rejects_bad_documents() {
    assert!(matches!(
        decode_network_headers(b"not json"),
        Err(DecodeError::Json(_))
    ));
    assert!(matches!(
        decode_network_headers(b"[]"),
        Err(DecodeError::NotAnObject)
    ));
    assert!(matches!(
        decode_network_headers(br#"{"host":1}"#),
        Err(DecodeError::NotText { .. })
    ));
    assert!(matches!(
        decode_network_headers(br#"{"x_forwarded_for":{"a":1}}"#),
        Err(DecodeError::NotText { .. })
    ));
    assert!(matches!(
        decode_network_headers(br#"{"trusted_proxy_ranges":5}"#),
        Err(DecodeError::NotRanges { .. })
    ));
    assert!(matches!(
        decode_network_headers(br#"{"mcp_trusted_proxy_ranges":"10.0.0.0/8"}"#),
        Err(DecodeError::NotRanges { .. })
    ));
    assert!(matches!(
        decode_network_headers(br#"{"mcp_trusted_proxy_ranges":["10.0.0.0/8",7]}"#),
        Err(DecodeError::NotText { .. })
    ));
    match decode_network_headers(br#"{"host":"h","nope":"x"}"#) {
        Err(DecodeError::UnknownField { field }) => assert_eq!(field, "nope"),
        other => panic!("expected an unknown-field error, got {other:?}"),
    }
}

#[test]
fn native_new_otherwise_unbounded_byte_decoders_reject_over_1048576_bytes_or_jso() {
    let prefix = br#"{"host":""#;
    let suffix = br#""}"#;
    let fill = MAX_NETWORK_HEADER_BYTES - prefix.len() - suffix.len();
    let mut document = Vec::with_capacity(MAX_NETWORK_HEADER_BYTES);
    document.extend_from_slice(prefix);
    document.resize(MAX_NETWORK_HEADER_BYTES - suffix.len(), b'x');
    document.extend_from_slice(suffix);
    assert_eq!(document.len(), MAX_NETWORK_HEADER_BYTES);
    let decoded = decode_network_headers(&document).unwrap();
    assert_eq!(decoded.host.as_deref().map(str::len), Some(fill));

    document.push(b' ');
    assert_eq!(document.len(), MAX_NETWORK_HEADER_BYTES + 1);
    assert!(matches!(
        decode_network_headers(&document),
        Err(DecodeError::TooLarge { size, limit })
            if size == MAX_NETWORK_HEADER_BYTES + 1 && limit == MAX_NETWORK_HEADER_BYTES
    ));

    // the depth limit counts the object itself; 64 levels parse, 65 are refused
    let open_inside = "[".repeat(MAX_NETWORK_HEADER_DEPTH - 1);
    let close_inside = "]".repeat(MAX_NETWORK_HEADER_DEPTH - 1);
    let inside = format!("{{\"host\":{open_inside}{close_inside}}}");
    assert!(matches!(
        decode_network_headers(inside.as_bytes()),
        Err(DecodeError::NotText { .. })
    ));
    let open_over = "[".repeat(MAX_NETWORK_HEADER_DEPTH);
    let close_over = "]".repeat(MAX_NETWORK_HEADER_DEPTH);
    let over = format!("{{\"host\":{open_over}{close_over}}}");
    assert!(matches!(
        decode_network_headers(over.as_bytes()),
        Err(DecodeError::TooDeep { depth, limit })
            if depth == MAX_NETWORK_HEADER_DEPTH + 1 && limit == MAX_NETWORK_HEADER_DEPTH
    ));
}

#[rstest]
fn replays_the_trusted_network_corpus(#[files("fuzz/corpus/trusted_network/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus seed is readable");
    drop(decode_network_headers(&bytes));
}
