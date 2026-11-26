from collections.abc import Callable
from typing import Literal, TypeAlias, TypedDict

from fastapi import Request
from litellm.proxy.auth.ip_address_utils import IPAddressUtils
from litellm.proxy.auth.network import (
    TrustedProxyConfig,
    ip_in_networks,
    normalize_cidr_ranges,
    parse_trusted_proxy_ranges,
    resolve_client_ip,
    resolve_network_context,
)
from litellm.proxy.auth.trusted_proxy_utils import (
    get_trusted_proxy_cidrs,
    require_trusted_proxy_request,
)
from starlette.types import Scope

ScenarioKind: TypeAlias = Literal[
    "normalize_cidr_ranges",
    "parse_trusted_proxy_ranges",
    "parse_internal_networks",
    "parse_trusted_proxy_networks",
    "ip_in_networks",
    "is_trusted_proxy",
    "is_internal_ip",
    "is_request_from_trusted_proxy",
    "is_request_https",
    "extract_client_ip_from_xff_hops",
    "get_mcp_client_ip",
    "resolve_client_ip",
    "resolve_network_context",
    "get_trusted_proxy_cidrs",
    "require_trusted_proxy_request",
]


class Headers(TypedDict):
    x_forwarded_for: str | None
    x_forwarded_proto: str | None
    host: str | None


class Settings(TypedDict):
    proxy_base_url: str | None
    use_x_forwarded_for: bool
    trusted_proxy_ranges: str | list[str] | None
    mcp_trusted_proxy_ranges: list[str] | None
    mcp_internal_ip_ranges: list[str] | None
    mcp_xff_num_trusted_hops: int | None


class Scenario(TypedDict):
    kind: ScenarioKind
    client_ip: str | None
    xff_header: str
    num_trusted_hops: int
    feature_name: str
    setting_name: str


class Input(TypedDict):
    headers: Headers
    peer: str | None
    scheme: str
    settings: Settings
    scenario: Scenario


class Resolution(TypedDict):
    client_ip: str | None
    via_trusted_proxy: bool


class Context(TypedDict):
    client_ip: str | None
    host: str | None
    via_trusted_proxy: bool


Outcome: TypeAlias = Resolution | Context | list[str] | bool | str | None


def build_request(input_value: Input) -> Request:
    headers = input_value["headers"]
    raw_headers: list[tuple[bytes, bytes]] = []
    for name, value in (
        (b"host", headers["host"]),
        (b"x-forwarded-for", headers["x_forwarded_for"]),
        (b"x-forwarded-proto", headers["x_forwarded_proto"]),
    ):
        if value is not None:
            raw_headers.append((name, value.encode("latin-1")))
    peer = input_value["peer"]
    client: tuple[str, int] | None = None
    if peer is not None:
        client = (peer, 0)
    scope: Scope = {
        "type": "http",
        "path": "/",
        "scheme": input_value["scheme"],
        "headers": raw_headers,
        "client": client,
    }
    return Request(scope=scope)


def proxy_config(input_value: Input) -> TrustedProxyConfig:
    ranges = input_value["settings"]["trusted_proxy_ranges"]
    cidrs = get_trusted_proxy_cidrs(general_settings={"trusted_proxy_ranges": ranges})
    return TrustedProxyConfig(use_forwarded_for=bool(cidrs), trusted_proxy_cidrs=cidrs)


def normalize_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["trusted_proxy_ranges"]
    return normalize_cidr_ranges(ranges)


def parse_trusted_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["trusted_proxy_ranges"]
    networks = parse_trusted_proxy_ranges(ranges)
    return [str(network) for network in networks]


def parse_internal_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["mcp_internal_ip_ranges"]
    networks = IPAddressUtils.parse_internal_networks(ranges)
    return [str(network) for network in networks]


def parse_proxies_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["mcp_trusted_proxy_ranges"]
    networks = IPAddressUtils.parse_trusted_proxy_networks(ranges)
    return [str(network) for network in networks]


def ip_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["trusted_proxy_ranges"]
    networks = parse_trusted_proxy_ranges(ranges)
    return ip_in_networks(input_value["scenario"]["client_ip"], networks)


def trusted_proxy_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["mcp_trusted_proxy_ranges"]
    networks = IPAddressUtils.parse_trusted_proxy_networks(ranges)
    return IPAddressUtils.is_trusted_proxy(input_value["scenario"]["client_ip"], networks)


def internal_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["mcp_internal_ip_ranges"]
    networks = IPAddressUtils.parse_internal_networks(ranges)
    return IPAddressUtils.is_internal_ip(input_value["scenario"]["client_ip"], networks)


def trusted_request_answer(input_value: Input) -> Outcome:
    request = build_request(input_value)
    settings = input_value["settings"]
    return IPAddressUtils.is_request_from_trusted_proxy(request, general_settings=settings)


def https_answer(input_value: Input) -> Outcome:
    request = build_request(input_value)
    settings = input_value["settings"]
    return IPAddressUtils.is_request_https(request, general_settings=settings)


def extract_answer(input_value: Input) -> Outcome:
    scenario = input_value["scenario"]
    hops = scenario["num_trusted_hops"]
    return IPAddressUtils.extract_client_ip_from_xff_hops(scenario["xff_header"], hops)


def mcp_client_answer(input_value: Input) -> Outcome:
    settings = input_value["settings"]
    mcp_settings = {
        "use_x_forwarded_for": settings["use_x_forwarded_for"],
        "mcp_trusted_proxy_ranges": settings["mcp_trusted_proxy_ranges"],
        "mcp_xff_num_trusted_hops": settings["mcp_xff_num_trusted_hops"],
    }
    return IPAddressUtils.get_mcp_client_ip(
        build_request(input_value), general_settings=mcp_settings
    )


def client_ip_answer(input_value: Input) -> Outcome:
    request = build_request(input_value)
    client_ip, via_trusted_proxy = resolve_client_ip(request, proxy_config(input_value))
    return Resolution(client_ip=client_ip, via_trusted_proxy=via_trusted_proxy)


def context_answer(input_value: Input) -> Outcome:
    request = build_request(input_value)
    context = resolve_network_context(request, proxy_config(input_value))
    return Context(
        client_ip=context.client_ip,
        host=context.host,
        via_trusted_proxy=context.via_trusted_proxy,
    )


def cidrs_answer(input_value: Input) -> Outcome:
    ranges = input_value["settings"]["trusted_proxy_ranges"]
    return get_trusted_proxy_cidrs(general_settings={"trusted_proxy_ranges": ranges})


def require_answer(input_value: Input) -> Outcome:
    settings = input_value["settings"]
    scenario = input_value["scenario"]
    require_trusted_proxy_request(
        request=build_request(input_value),
        general_settings={
            "trusted_proxy_ranges": settings["trusted_proxy_ranges"],
            "mcp_trusted_proxy_ranges": settings["mcp_trusted_proxy_ranges"],
        },
        feature_name=scenario["feature_name"],
        setting_name=scenario["setting_name"],
    )
    return None


Answer: TypeAlias = Callable[[Input], Outcome]

ANSWER_BY_KIND: dict[ScenarioKind, Answer] = {
    "normalize_cidr_ranges": normalize_answer,
    "parse_trusted_proxy_ranges": parse_trusted_answer,
    "parse_internal_networks": parse_internal_answer,
    "parse_trusted_proxy_networks": parse_proxies_answer,
    "ip_in_networks": ip_answer,
    "is_trusted_proxy": trusted_proxy_answer,
    "is_internal_ip": internal_answer,
    "is_request_from_trusted_proxy": trusted_request_answer,
    "is_request_https": https_answer,
    "extract_client_ip_from_xff_hops": extract_answer,
    "get_mcp_client_ip": mcp_client_answer,
    "resolve_client_ip": client_ip_answer,
    "resolve_network_context": context_answer,
    "get_trusted_proxy_cidrs": cidrs_answer,
    "require_trusted_proxy_request": require_answer,
}


def run(input_value: Input) -> Outcome:
    kind = input_value["scenario"]["kind"]
    return ANSWER_BY_KIND[kind](input_value)
