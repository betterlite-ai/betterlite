import os
from types import SimpleNamespace
from typing import Final, Literal, NotRequired, TypedDict

from litellm.proxy._experimental.mcp_server import utils
from litellm.types.mcp import MCPTransport
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter

SHORT_FLAG_ENV: Final = "LITELLM_USE_SHORT_MCP_TOOL_PREFIX"
BASE_URL_ENV: Final = "PROXY_BASE_URL"


class ServerSpec(TypedDict):
    server_id: str
    alias: NotRequired[str | None]
    server_name: NotRequired[str | None]
    spec_path: NotRequired[str | None]


class PrefixNameInput(TypedDict):
    function: Literal["add_server_prefix_to_name"]
    name: str
    server_name: str


class ShortFlagInput(TypedDict):
    function: Literal["short_tool_prefix_enabled"]
    raw: NotRequired[str | None]


class MatchToolInput(TypedDict):
    function: Literal["match_known_tool_name"]
    tool_name: str
    server: ServerSpec
    names: list[str]
    short_prefix_enabled: NotRequired[bool]


class MatchPrefixInput(TypedDict):
    function: Literal["match_known_server_prefix"]
    name: str
    known_prefixes: list[str]


class PrefixedInput(TypedDict):
    function: Literal["is_tool_name_prefixed"]
    tool_name: str
    known_prefixes: NotRequired[list[str] | None]


class AdminEnvInput(TypedDict):
    function: Literal["parse_admin_env_vars"]
    env_vars: NotRequired[list[JsonValue] | None]


class ReferencesInput(TypedDict):
    function: Literal["env_references"]
    value: str
    strings: list[str]


class InterpolateInput(TypedDict):
    function: Literal["interpolate"]
    value: str
    headers: dict[str, str]
    variables: dict[str, str]


class SetupUrlInput(TypedDict):
    function: Literal["setup_url"]
    server_id: str
    proxy_base_url: NotRequired[str | None]


class MissingVarsInput(TypedDict):
    function: Literal["missing_vars_message"]
    server_id: str
    server_name: NotRequired[str | None]
    missing: list[str]
    setup_url: str


class AllowlistInput(TypedDict):
    function: Literal["allowlist"]
    mcp_info: NotRequired[dict[str, JsonValue] | None]
    allowed_tools: NotRequired[list[str] | None]


class ToolResultInput(TypedDict):
    function: Literal["tool_result_error"]
    result: JsonValue


class MergeHeadersInput(TypedDict):
    function: Literal["merge_headers"]
    extra_headers: NotRequired[dict[str, str] | None]
    static_headers: NotRequired[dict[str, str] | None]


class SyntheticInput(TypedDict):
    function: Literal["synthetic"]
    path: str
    raw_headers: NotRequired[list[tuple[str, str]] | None]
    client_ip: NotRequired[str | None]


class SyntheticScope(TypedDict):
    method: str
    path: str
    scheme: str
    server: tuple[str, int]
    query_string: bytes
    root_path: str
    headers: list[tuple[bytes, bytes]]
    client: NotRequired[tuple[str, int]]


NamesInput = PrefixNameInput | ShortFlagInput | MatchToolInput | MatchPrefixInput | PrefixedInput
EnvsInput = AdminEnvInput | ReferencesInput | InterpolateInput | SetupUrlInput | MissingVarsInput
WireInput = AllowlistInput | ToolResultInput | MergeHeadersInput | SyntheticInput
Input = NamesInput | EnvsInput | WireInput
Output = JsonValue | dict[str, str] | list[str]

SCOPE: Final[TypeAdapter[SyntheticScope]] = TypeAdapter(SyntheticScope)
ENV_PAIR: Final = TypeAdapter(tuple[dict[str, JsonValue], list[JsonValue]])


def set_env(name: str, value: str | None) -> None:
    if value is not None:
        os.environ[name] = value
        return
    if name in os.environ:
        del os.environ[name]


def build_server(spec: ServerSpec) -> MCPServer:
    return MCPServer(
        server_id=spec["server_id"],
        name=spec["server_id"],
        transport=MCPTransport.http,
        alias=spec.get("alias"),
        server_name=spec.get("server_name"),
        spec_path=spec.get("spec_path"),
    )


def run(input_value: Input) -> Output:
    match input_value["function"]:
        case (
            "add_server_prefix_to_name"
            | "short_tool_prefix_enabled"
            | "match_known_tool_name"
            | "match_known_server_prefix"
            | "is_tool_name_prefixed"
        ):
            return run_names(input_value)
        case (
            "parse_admin_env_vars"
            | "env_references"
            | "interpolate"
            | "setup_url"
            | "missing_vars_message"
        ):
            return run_envs(input_value)
        case "allowlist" | "tool_result_error" | "merge_headers" | "synthetic":
            return run_wire(input_value)


def run_names(call: NamesInput) -> Output:
    match call["function"]:
        case "add_server_prefix_to_name":
            return utils.add_server_prefix_to_name(call["name"], call["server_name"])
        case "short_tool_prefix_enabled":
            set_env(SHORT_FLAG_ENV, call.get("raw"))
            return utils.is_short_mcp_tool_prefix_enabled()
        case "match_known_tool_name":
            set_env(SHORT_FLAG_ENV, "true" if call.get("short_prefix_enabled") else None)
            return utils.match_known_tool_name(
                call["tool_name"], build_server(call["server"]), call["names"]
            )
        case "match_known_server_prefix":
            return match_prefix(call)
        case "is_tool_name_prefixed":
            known = call.get("known_prefixes")
            prefixes = frozenset(known) if known is not None else None
            return utils.is_tool_name_prefixed(call["tool_name"], prefixes)


def run_envs(call: EnvsInput) -> Output:
    match call["function"]:
        case "parse_admin_env_vars":
            return admin_env(call)
        case "env_references":
            return references(call)
        case "interpolate":
            return interpolate(call)
        case "setup_url":
            set_env(BASE_URL_ENV, call.get("proxy_base_url"))
            return utils.build_env_var_setup_url(call["server_id"])
        case "missing_vars_message":
            return missing_message(call)


def run_wire(call: WireInput) -> Output:
    match call["function"]:
        case "allowlist":
            server = SimpleNamespace(
                mcp_info=call.get("mcp_info"), allowed_tools=call.get("allowed_tools")
            )
            return {
                "enforced": utils.is_server_tool_allowlist_enforced(server),
                "applies": utils.server_applies_tool_allowlist(server),
            }
        case "tool_result_error":
            return utils.extract_mcp_tool_result_error_message(call["result"])
        case "merge_headers":
            return utils.merge_mcp_headers(
                extra_headers=call.get("extra_headers"),
                static_headers=call.get("static_headers"),
            )
        case "synthetic":
            return synthetic(call)


def match_prefix(call: MatchPrefixInput) -> list[str] | None:
    matched = utils.match_known_server_prefix(call["name"], call["known_prefixes"])
    return list(matched) if matched is not None else None


def admin_env(call: AdminEnvInput) -> dict[str, JsonValue]:
    pair = ENV_PAIR.validate_python(utils.parse_admin_env_vars(call.get("env_vars")))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/utils.py#L671-L673
    return {"globals": pair[0], "users": pair[1]}


def references(call: ReferencesInput) -> dict[str, JsonValue]:
    references: list[JsonValue] = []
    references.extend(sorted(utils.find_env_var_references(call["value"])))
    collected: list[JsonValue] = []
    collected.extend(sorted(utils.collect_env_var_references(strings=call["strings"])))
    return {"references": references, "collected": collected}


def interpolate(call: InterpolateInput) -> dict[str, JsonValue]:
    interpolated = utils.interpolate_headers(call["headers"], call["variables"])
    headers: dict[str, JsonValue] = dict[str, JsonValue](interpolated.items())
    value = utils.interpolate_env_vars(call["value"], call["variables"])
    return {"value": value, "headers": headers}


def missing_message(call: MissingVarsInput) -> str:
    error = utils.MCPMissingUserEnvVarsError(
        server_id=call["server_id"],
        server_name=call.get("server_name"),
        missing=call["missing"],
        setup_url=call["setup_url"],
    )
    return str(error)


def synthetic(call: SyntheticInput) -> dict[str, JsonValue]:
    raw = call.get("raw_headers")
    request = utils.build_synthetic_mcp_request(
        path=call["path"],
        raw_headers=dict(raw) if raw is not None else None,
        client_ip=call.get("client_ip"),
    )
    scope = SCOPE.validate_python(dict(request.scope))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/utils.py#L1007-L1057
    return synthetic_output(scope)


def byte_values(raw: bytes) -> list[JsonValue]:
    values: list[JsonValue] = []
    values.extend(raw)
    return values


def synthetic_output(scope: SyntheticScope) -> dict[str, JsonValue]:
    headers: list[JsonValue] = [
        [byte_values(name), byte_values(value)] for name, value in scope["headers"]
    ]
    client = scope.get("client")
    return {
        "method": scope["method"],
        "path": scope["path"],
        "scheme": scope["scheme"],
        "server": [scope["server"][0], scope["server"][1]],
        "query_string": byte_values(scope["query_string"]),
        "root_path": scope["root_path"],
        "headers": headers,
        "client": [client[0], client[1]] if client is not None else None,
    }
