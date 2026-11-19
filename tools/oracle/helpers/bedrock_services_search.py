import os
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
import litellm
from litellm.llms.bedrock.search.transformation import AgentCoreSearchConfig
from pydantic import JsonValue

ERROR_KINDS: Final[tuple[tuple[str, str], ...]] = (
    ("AGENTCORE_GATEWAY_URL is not set", "gateway_url_missing"),
    ("Invalid AgentCore search tool_name", "tool_name"),
    ("Refusing to send AgentCore credentials over plaintext HTTP", "plaintext_transport"),
    ("Refusing to send SigV4-signed AgentCore requests", "untrusted_signing_host"),
    ("Cannot derive the SigV4 signing region", "signing_region"),
    ("Invalid AWS region format", "invalid_region"),
    ("Refusing to send the server-configured", "credential"),
)


class GatewayRun(TypedDict):
    api_base: str | None
    gateway_url: str | None


class GatewayScenario(TypedDict):
    scenario: Literal["configured_gateway_required"]
    runs: list[GatewayRun]


class HeaderRun(TypedDict):
    headers: dict[str, str]
    protocol_version: str | None


class HeaderScenario(TypedDict):
    scenario: Literal["mcp_request_headers"]
    runs: list[HeaderRun]


class ToolRun(TypedDict):
    query: str | list[str]
    optional_params: dict[str, JsonValue]
    configured_tool_name: str | None


class ToolSuffixScenario(TypedDict):
    scenario: Literal["websearch_tool_suffix"]
    runs: list[ToolRun]


class QueryCapScenario(TypedDict):
    scenario: Literal["query_list_and_200_char_cap"]
    runs: list[ToolRun]


class CredentialRun(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None


class SignRun(TypedDict):
    api_base: str
    headers: dict[str, str]
    optional_params: dict[str, JsonValue]
    request_data: JsonValue
    api_key: str | None
    gateway_url: str | None
    gateway_token: str | None
    credentials: CredentialRun
    session_region: str | None
    now: int


class TransportScenario(TypedDict):
    scenario: Literal["https_or_loopback_only"]
    runs: list[SignRun]


class BearerScenario(TypedDict):
    scenario: Literal["gateway_bearer_trust"]
    runs: list[SignRun]


class HostScenario(TypedDict):
    scenario: Literal["gateway_host_signing"]
    runs: list[SignRun]


class RegionScenario(TypedDict):
    scenario: Literal["gateway_signing_region"]
    runs: list[SignRun]


Input = (
    GatewayScenario
    | HeaderScenario
    | ToolSuffixScenario
    | QueryCapScenario
    | TransportScenario
    | BearerScenario
    | HostScenario
    | RegionScenario
)


class AgentCoreApi(Protocol):
    def validate_environment(self, headers: dict[str, str]) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, optional_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_search_request(
        self, query: str | list[str], optional_params: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...

    def sign_request(
        self,
        headers: dict[str, str],
        optional_params: dict[str, JsonValue],
        request_data: JsonValue,
        api_base: str,
        api_key: str | None,
    ) -> tuple[dict[str, str], bytes | None]: ...


CONFIG: Final[AgentCoreApi] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/search/transformation.py#L158-L160
    "AgentCoreApi", AgentCoreSearchConfig()
)

litellm.suppress_debug_info = True


class FrozenClock:
    def __init__(self, frozen: datetime) -> None:
        self.frozen = frozen

    def __call__(self) -> datetime:
        return self.frozen


def _error_kind(message: str) -> str:
    for prefix, kind in ERROR_KINDS:
        if message.startswith(prefix):
            return kind
    raise ValueError(message)


def _gateway_value(case: GatewayRun) -> JsonValue:
    os.environ.pop("AGENTCORE_GATEWAY_URL", None)
    configured = case["gateway_url"]
    if configured is not None:
        os.environ["AGENTCORE_GATEWAY_URL"] = configured
    try:
        url = CONFIG.get_complete_url(case["api_base"], {})
    except ValueError as error:
        return {"api_base": case["api_base"], "ok": False, "error": _error_kind(str(error))}
    return {"api_base": case["api_base"], "ok": True, "url": url}


def _header_value(case: HeaderRun) -> JsonValue:
    os.environ.pop("AGENTCORE_MCP_PROTOCOL_VERSION", None)
    version = case["protocol_version"]
    if version is not None:
        os.environ["AGENTCORE_MCP_PROTOCOL_VERSION"] = version
    merged = CONFIG.validate_environment(case["headers"])
    converted: dict[str, JsonValue] = dict(merged)
    return {"ok": True, "headers": converted}


def _request_value(case: ToolRun) -> JsonValue:
    os.environ.pop("AGENTCORE_SEARCH_TOOL_NAME", None)
    configured = case["configured_tool_name"]
    if configured is not None:
        os.environ["AGENTCORE_SEARCH_TOOL_NAME"] = configured
    try:
        body = CONFIG.transform_search_request(case["query"], case["optional_params"])
    except ValueError as error:
        return {"ok": False, "error": _error_kind(str(error))}
    return {"ok": True, "body": body}


def _signing_params(case: SignRun) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = dict(case["optional_params"])
    keys = case["credentials"]
    params["aws_access_key_id"] = keys["access_key_id"]
    params["aws_secret_access_key"] = keys["secret_access_key"]
    if keys["session_token"] is not None:
        params["aws_session_token"] = keys["session_token"]
    return params


def _apply_environment(case: SignRun) -> None:
    gateway_url = case["gateway_url"]
    if gateway_url is not None:
        os.environ["AGENTCORE_GATEWAY_URL"] = gateway_url
    gateway_token = case["gateway_token"]
    if gateway_token is not None:
        os.environ["AGENTCORE_GATEWAY_TOKEN"] = gateway_token
    session_region = case["session_region"]
    if session_region:
        os.environ["AWS_DEFAULT_REGION"] = session_region


def _sign_value(case: SignRun) -> JsonValue:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull
    _apply_environment(case)
    frozen = datetime.fromtimestamp(case["now"], tz=UTC).replace(tzinfo=None)
    botocore.auth.get_current_datetime = FrozenClock(frozen)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L1691
    try:
        headers, body = CONFIG.sign_request(
            case["headers"],
            _signing_params(case),
            case["request_data"],
            case["api_base"],
            case["api_key"],
        )
    except TypeError:
        return {"ok": False, "error": "request_body"}
    except ValueError as error:
        return {"ok": False, "error": _error_kind(str(error))}
    if body is None:
        raise ValueError("sign_request returned no body")
    converted: dict[str, JsonValue] = dict(headers)
    return {
        "ok": True,
        "method": "POST",
        "url": case["api_base"],
        "headers": converted,
        "body": body.decode(),
    }


def run(case: Input) -> JsonValue:
    if case["scenario"] == "configured_gateway_required":
        return {"results": [_gateway_value(item) for item in case["runs"]]}
    if case["scenario"] == "mcp_request_headers":
        return {"results": [_header_value(item) for item in case["runs"]]}
    if case["scenario"] == "websearch_tool_suffix":
        return {"results": [_request_value(item) for item in case["runs"]]}
    if case["scenario"] == "query_list_and_200_char_cap":
        return {"results": [_request_value(item) for item in case["runs"]]}
    return {"results": [_sign_value(item) for item in case["runs"]]}
