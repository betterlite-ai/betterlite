import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.pass_through_endpoints.common_utils import get_litellm_virtual_key
from litellm.proxy.pass_through_endpoints.llm_passthrough_endpoints import _caller_headers_without_litellm_secrets, _forwarded_headers_for_credentialless_vertex_passthrough, _normalize_credential_value, _upstream_headers_for_anthropic_route, _upstream_headers_for_bedrock_agent_runtime_route  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_passthrough_endpoints.py#L2460-L2600
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
    def __init__(
        self,
        master_key: str | None,
        general_settings: dict[str, JsonValue],
        user_custom_auth: str | None,
    ) -> None:
        super().__init__(PROXY_MODULE)
        self.master_key = master_key
        self.general_settings = general_settings
        self.user_custom_auth = user_custom_auth


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
    custom_auth = CUSTOM_AUTH if settings["custom_auth_configured"] else None
    proxy = ProxyServer(
        settings["master_key"],
        general_settings_of(settings),
        custom_auth,
    )
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


def virtual_key_answer(call: VirtualKeyCall) -> JsonValue:
    key = get_litellm_virtual_key(request_of(call["headers"]))
    return {"value": key}


def normalize_answer(call: NormalizeCall) -> JsonValue:
    return {
        "value": _normalize_credential_value(call["value"]),
    }


def filter_answer(call: FilterCall) -> JsonValue:
    install_proxy(call["settings"])
    forwarded = _caller_headers_without_litellm_secrets(
        request_of(call["headers"]),
        caller_of(call["caller"]),
        frozenset(call["never_forwarded"]),
    )
    return {"headers": JSON.validate_python(dict(forwarded))}


def vertex_answer(call: ProviderCall) -> JsonValue:
    install_proxy(call["settings"])
    forwarded = _forwarded_headers_for_credentialless_vertex_passthrough(
        request_of(call["headers"]),
        caller_of(call["caller"]),
    )
    return {"headers": JSON.validate_python(dict(forwarded))}


def anthropic_answer(call: AnthropicCall) -> JsonValue:
    install_proxy(call["settings"])
    proxy = call["proxy_auth_header"]
    forwarded = _upstream_headers_for_anthropic_route(
        request_of(call["headers"]),
        caller_of(call["caller"]),
        None if proxy is None else dict(proxy),
    )
    return {"headers": JSON.validate_python(dict(forwarded))}


def bedrock_answer(call: BedrockCall) -> JsonValue:
    install_proxy(call["settings"])
    forwarded = _upstream_headers_for_bedrock_agent_runtime_route(
        request_of(call["headers"]),
        caller_of(call["caller"]),
        dict(call["signed_headers"]),
    )
    return {"headers": JSON.validate_python(dict(forwarded))}


def run(input_value: Input) -> JsonValue:
    if "virtual_key" in input_value:
        return virtual_key_answer(input_value["virtual_key"])
    if "normalize" in input_value:
        return normalize_answer(input_value["normalize"])
    if "filter" in input_value:
        return filter_answer(input_value["filter"])
    if "vertex" in input_value:
        return vertex_answer(input_value["vertex"])
    if "anthropic" in input_value:
        return anthropic_answer(input_value["anthropic"])
    if "bedrock" in input_value:
        return bedrock_answer(input_value["bedrock"])
    raise ValueError("the input names no helper operation")
