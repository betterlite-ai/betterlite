use betterlite_auth::trusted_network::cidr::{
    CidrRanges, ip_in_networks, normalize_cidr_ranges, parse_trusted_proxy_ranges,
};
use betterlite_auth::trusted_network::internal::{
    is_internal_ip, is_trusted_proxy, parse_internal_networks, parse_trusted_proxy_networks,
};
use betterlite_auth::trusted_network::proxy_utils::{
    get_trusted_proxy_cidrs, principal_proxy_config, require_trusted_proxy_request,
};
use betterlite_auth::trusted_network::request::{
    NetworkRequest, extract_client_ip_from_xff_hops, get_mcp_client_ip,
    is_request_from_trusted_proxy, is_request_https, resolve_client_ip, resolve_network_context,
};
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const TRUSTED_PROXY: &str = "trusted_proxy_cidrs";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    headers: Headers,
    peer: Option<String>,
    scheme: String,
    settings: Settings,
    scenario: Scenario,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Headers {
    x_forwarded_for: Option<String>,
    x_forwarded_proto: Option<String>,
    host: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    proxy_base_url: Option<String>,
    #[serde(default)]
    use_x_forwarded_for: bool,
    trusted_proxy_ranges: Option<CidrRanges>,
    mcp_trusted_proxy_ranges: Option<Vec<String>>,
    mcp_internal_ip_ranges: Option<Vec<String>>,
    mcp_xff_num_trusted_hops: Option<i64>,
}

// the owned call site to exercise; the arguments travel with it, empty where the call takes none
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    kind: Kind,
    client_ip: Option<String>,
    xff_header: String,
    num_trusted_hops: i64,
    feature_name: String,
    setting_name: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    NormalizeCidrRanges,
    ParseTrustedProxyRanges,
    ParseInternalNetworks,
    ParseTrustedProxyNetworks,
    IpInNetworks,
    IsTrustedProxy,
    IsInternalIp,
    #[serde(rename = "is_request_from_trusted_proxy")]
    TrustedRequest,
    IsRequestHttps,
    #[serde(rename = "extract_client_ip_from_xff_hops")]
    ExtractHops,
    GetMcpClientIp,
    ResolveClientIp,
    ResolveNetworkContext,
    GetTrustedProxyCidrs,
    RequireTrustedProxyRequest,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = trusted_settings(&input.settings);
        let request = network_request(input);
        let scenario = &input.scenario;
        let ranges = input.settings.trusted_proxy_ranges.as_ref();
        let texts = parse_trusted_proxy_ranges(ranges, TRUSTED_PROXY);
        let proxies =
            parse_trusted_proxy_networks(input.settings.mcp_trusted_proxy_ranges.as_deref());
        let inside = parse_internal_networks(input.settings.mcp_internal_ip_ranges.as_deref());
        let client = scenario.client_ip.as_deref();
        let xff = scenario.xff_header.as_str();
        let hops = scenario.num_trusted_hops;
        let feature = scenario.feature_name.as_str();
        let setting_name = scenario.setting_name.as_str();
        match scenario.kind {
            Kind::NormalizeCidrRanges => Ok(json!(normalize_cidr_ranges(ranges))),
            Kind::ParseTrustedProxyRanges => Ok(json!(rendered(&texts))),
            Kind::ParseInternalNetworks => Ok(json!(rendered(&inside))),
            Kind::ParseTrustedProxyNetworks => Ok(json!(rendered(&proxies))),
            Kind::IpInNetworks => Ok(json!(ip_in_networks(client, &texts))),
            Kind::IsTrustedProxy => Ok(json!(is_trusted_proxy(client, &proxies))),
            Kind::IsInternalIp => Ok(json!(is_internal_ip(client, Some(&inside)))),
            Kind::TrustedRequest => Ok(json!(is_request_from_trusted_proxy(&request, &settings))),
            Kind::IsRequestHttps => Ok(json!(is_request_https(&request, &settings))),
            Kind::ExtractHops => Ok(json!(extract_client_ip_from_xff_hops(xff, hops))),
            Kind::GetMcpClientIp => Ok(json!(get_mcp_client_ip(&request, &settings))),
            Kind::ResolveClientIp => Ok(resolved_client(&request, &settings)),
            Kind::ResolveNetworkContext => Ok(resolved_context(&request, &settings)),
            Kind::GetTrustedProxyCidrs => Ok(json!(get_trusted_proxy_cidrs(&settings))),
            Kind::RequireTrustedProxyRequest => {
                require_trusted(&request, &settings, feature, setting_name)
            }
        }
    }
}

fn trusted_settings(given: &Settings) -> TrustedNetworkSettings {
    TrustedNetworkSettings {
        proxy_base_url: given.proxy_base_url.clone(),
        use_x_forwarded_for: given.use_x_forwarded_for,
        trusted_proxy_ranges: given.trusted_proxy_ranges.clone(),
        mcp_trusted_proxy_ranges: given.mcp_trusted_proxy_ranges.clone(),
        mcp_xff_num_trusted_hops: given.mcp_xff_num_trusted_hops,
    }
}

fn network_request<'request>(input: &'request Input) -> NetworkRequest<'request> {
    NetworkRequest {
        peer: input.peer.as_deref(),
        x_forwarded_for: input.headers.x_forwarded_for.as_deref(),
        x_forwarded_proto: input.headers.x_forwarded_proto.as_deref(),
        host: input.headers.host.as_deref(),
        scheme: &input.scheme,
    }
}

fn require_trusted(
    request: &NetworkRequest<'_>,
    settings: &TrustedNetworkSettings,
    feature_name: &str,
    setting_name: &str,
) -> HelperResult {
    require_trusted_proxy_request(request, settings, feature_name, setting_name)
        .or_value_error()?;
    Ok(json!(null))
}

fn resolved_client(request: &NetworkRequest<'_>, settings: &TrustedNetworkSettings) -> Value {
    let config = principal_proxy_config(settings);
    let (client_ip, via_trusted_proxy) = resolve_client_ip(request, &config);
    json!({
        "client_ip": client_ip,
        "via_trusted_proxy": via_trusted_proxy,
    })
}

fn resolved_context(request: &NetworkRequest<'_>, settings: &TrustedNetworkSettings) -> Value {
    let config = principal_proxy_config(settings);
    let context = resolve_network_context(request, &config);
    json!({
        "client_ip": context.client_ip,
        "host": context.host,
        "via_trusted_proxy": context.via_trusted_proxy,
    })
}

// Python renders a parsed network with str(), the same "address/prefix" text ipnet prints
fn rendered<N: std::fmt::Display>(networks: &[N]) -> Vec<String> {
    let mut texts = Vec::with_capacity(networks.len());
    for network in networks {
        texts.push(network.to_string());
    }
    texts
}
