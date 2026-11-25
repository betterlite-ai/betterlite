import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.pass_through_endpoints.common_utils import get_litellm_virtual_key
from litellm.proxy.pass_through_endpoints.llm_passthrough_endpoints import (
    _caller_headers_without_litellm_secrets,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2553-L2566
    _forwarded_headers_for_credentialless_vertex_passthrough,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2569-L2578
    _normalize_credential_value,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2460-L2472
    _upstream_headers_for_anthropic_route,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2581-L2589
    _upstream_headers_for_bedrock_agent_runtime_route,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2592-L2600
)
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
CUSTOM_AUTH: Final = "custom-auth"
JSON: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

litellm.suppress_debug_info = True

HeaderPair: TypeAlias = tuple[str, str]
AuthSwitch: TypeAlias = Literal[
    "enable_jwt_auth", "enable_oauth2_auth", "enable_oauth2_proxy_auth"
]


class Caller(TypedDict):
    stored_api_key: str | None
    jwt_claims: dict[str, JsonValue] | None


class Settings(TypedDict):
    master_key: str | None
    custom_auth_configured: bool
    auth_mode: list[AuthSwitch]
    litellm_key_header_name: str | None
    endpoint_headers: list[dict[str, JsonValue]]


class VirtualKeyCall(TypedDict):
    headers: list[HeaderPair]


class NormalizeCall(TypedDict):
    value: str


class FilterCall(TypedDict):
    headers: list[HeaderPair]
    never_forwarded: list[str]
    caller: Caller
    settings: Settings


class ProviderCall(TypedDict):
    headers: list[HeaderPair]
    caller: Caller
    settings: Settings


class AnthropicCall(TypedDict):
    headers: list[HeaderPair]
    caller: Caller
    settings: Settings
    proxy_auth_header: list[HeaderPair] | None


class BedrockCall(TypedDict):
    headers: list[HeaderPair]
    caller: Caller
    settings: Settings
    signed_headers: list[HeaderPair]


class Input(TypedDict):
    virtual_key: NotRequired[VirtualKeyCall]
    normalize: NotRequired[NormalizeCall]
    filter: NotRequired[FilterCall]
    vertex: NotRequired[ProviderCall]
    anthropic: NotRequired[AnthropicCall]
    bedrock: NotRequired[BedrockCall]


class ProxyServer(ModuleType):
    master_key: str | None
    general_settings: dict[str, JsonValue]
    user_custom_auth: str | None


def general_settings_of(settings: Settings) -> dict[str, JsonValue]:
    endpoints: list[JsonValue] = [
        {"headers": dict(headers)} for headers in settings["endpoint_headers"]
    ]
    return {
        "litellm_key_header_name": settings["litellm_key_header_name"],
        "pass_through_endpoints": endpoints,
        "enable_jwt_auth": "enable_jwt_auth" in settings["auth_mode"],
        "enable_oauth2_auth": "enable_oauth2_auth" in settings["auth_mode"],
        "enable_oauth2_proxy_auth": "enable_oauth2_proxy_auth" in settings["auth_mode"],
    }


def install_proxy(settings: Settings) -> None:
    proxy = ProxyServer(PROXY_MODULE)
    proxy.master_key = settings["master_key"]
    proxy.general_settings = general_settings_of(settings)
    proxy.user_custom_auth = CUSTOM_AUTH if settings["custom_auth_configured"] else None
    sys.modules[PROXY_MODULE] = proxy


def request_of(pairs: list[HeaderPair]) -> Request:
    raw: list[tuple[bytes, bytes]] = [
        (name.lower().encode("latin-1"), value.encode("latin-1"))
        for name, value in pairs
    ]
    scope: Scope = {"type": "http", "headers": raw}
    return Request(scope=scope)


def caller_of(caller: Caller) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        api_key=caller["stored_api_key"],
        jwt_claims=caller["jwt_claims"],
    )


def run(input_value: Input) -> JsonValue:
    if "virtual_key" in input_value:
        key = get_litellm_virtual_key(request_of(input_value["virtual_key"]["headers"]))
        return {"value": key}
    if "normalize" in input_value:
        return {"value": _normalize_credential_value(input_value["normalize"]["value"])}
    if "filter" in input_value:
        filter_call = input_value["filter"]
        install_proxy(filter_call["settings"])
        filtered = _caller_headers_without_litellm_secrets(
            request_of(filter_call["headers"]),
            caller_of(filter_call["caller"]),
            frozenset(filter_call["never_forwarded"]),
        )
        return {"headers": JSON.validate_python(filtered)}
    if "vertex" in input_value:
        vertex_call = input_value["vertex"]
        install_proxy(vertex_call["settings"])
        vertex_headers = _forwarded_headers_for_credentialless_vertex_passthrough(
            request_of(vertex_call["headers"]), caller_of(vertex_call["caller"])
        )
        return {"headers": JSON.validate_python(vertex_headers)}
    if "anthropic" in input_value:
        anthropic_call = input_value["anthropic"]
        install_proxy(anthropic_call["settings"])
        proxy_headers = anthropic_call["proxy_auth_header"]
        anthropic_headers = _upstream_headers_for_anthropic_route(
            request_of(anthropic_call["headers"]),
            caller_of(anthropic_call["caller"]),
            None if proxy_headers is None else dict(proxy_headers),
        )
        return {"headers": JSON.validate_python(anthropic_headers)}
    if "bedrock" in input_value:
        bedrock_call = input_value["bedrock"]
        install_proxy(bedrock_call["settings"])
        bedrock_headers = _upstream_headers_for_bedrock_agent_runtime_route(
            request_of(bedrock_call["headers"]),
            caller_of(bedrock_call["caller"]),
            dict(bedrock_call["signed_headers"]),
        )
        return {"headers": JSON.validate_python(bedrock_headers)}
    raise ValueError("the input names no helper operation")
