import asyncio
import itertools
from collections.abc import Callable, Mapping
from enum import Enum, StrEnum
from typing import Final

import httpx
import litellm.types.mcp as litellm_mcp  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp.py#L1
import litellm.types.mcp_server.mcp_server_manager as litellm_manager  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/mcp_server_manager.py#L1
import litellm.types.mcp_server.mcp_toolset as litellm_toolset  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/mcp_toolset.py#L1
import litellm.types.mcp_server.tool_registry as litellm_registry  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/tool_registry.py#L1
import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

import betterlite.types.mcp.credentials as betterlite_credentials
import betterlite.types.mcp.enums as betterlite_enums
import betterlite.types.mcp.headers as betterlite_headers
import betterlite.types.mcp.hooks as betterlite_hooks
import betterlite.types.mcp.models as betterlite_models
import betterlite.types.mcp_server.mcp_server_manager as betterlite_manager
import betterlite.types.mcp_server.mcp_toolset as betterlite_toolset
import betterlite.types.mcp_server.tool_registry as betterlite_registry

LITELLM_MCP_NAMES: Final = {
    "DEFAULT_CREDENTIAL_HEADER",
    "DEFAULT_SUBJECT_TOKEN_TYPE",
    "MCPAdvertisedVersions",
    "MCPAllowedClient",
    "MCPAuth",
    "MCPAuthType",
    "MCPCredentials",
    "MCPDuringCallRequestObject",
    "MCPDuringCallResponseObject",
    "MCPGatewaySession",
    "MCPGatewaySessionGroupCount",
    "MCPGatewaySessionsResponse",
    "MCPGatewaySessionsTerminateResponse",
    "MCPLegacyVersion",
    "MCPPostCallResponseObject",
    "MCPPreCallRequestObject",
    "MCPPreCallResponseObject",
    "MCPPublicServer",
    "MCPServerCostInfo",
    "MCPSpecVersion",
    "MCPSpecVersionType",
    "MCPStdioConfig",
    "MCPTokenEndpointAuthMethod",
    "MCPToolSearchSettings",
    "MCPTransport",
    "MCPTransportType",
    "MCPUpstreamProtocol",
    "MCP_ADMIN_CONFIG_CREDENTIAL_KEYS",
    "MCP_LEGACY_VERSIONS",
    "_DEFAULT_PORTS",
    "_HEADER_NAME_TOKEN",
    "credential_redirect_hook",
    "crosses_origin",
    "custom_credential_slot",
    "has_header",
    "normalize_upstream_header_name",
    "same_header",
    "without_header",
}
LITELLM_MANAGER_NAMES: Final = {
    "MCPInfo",
    "MCPOAuthIdentityBinding",
    "MCPOAuthMetadata",
    "MCPServer",
    "PinnedMCPTool",
    "_PINNED_TOOLS",
    "_validate_mcp_protocol_metadata",
    "parse_pinned_tools",
}
LITELLM_TOOLSET_NAMES: Final = {
    "MCPToolset",
    "MCPToolsetTool",
    "NewMCPToolsetRequest",
    "UpdateMCPToolsetRequest",
}
LITELLM_REGISTRY_NAMES: Final = {
    "CallToolRequest",
    "ContentItem",
    "ListToolsResponse",
    "MCPTool",
    "ToolSchema",
}
BETTERLITE_MCP_MODULES: Final = (
    betterlite_credentials,
    betterlite_enums,
    betterlite_headers,
    betterlite_hooks,
    betterlite_models,
)
# fields whose Any, object or mcp.types content in LiteLLM is a JSON value here
NARROWED_FIELDS: Final = frozenset(
    {
        "arguments",
        "env_vars",
        "hidden_params",
        "input_schema",
        "mcp_info",
        "mcp_tool_call_response",
        "modified_arguments",
        "params",
        "token_validation",
        "user_api_key_auth",
    }
)

ENUM_PAIRS: Final[list[tuple[type[StrEnum], type[Enum]]]] = [
    (betterlite_enums.MCPTransport, litellm_mcp.MCPTransport),
    (betterlite_enums.MCPSpecVersion, litellm_mcp.MCPSpecVersion),
    (betterlite_enums.MCPAuth, litellm_mcp.MCPAuth),
]
MODEL_PAIRS: Final[list[tuple[type[BaseModel], type[BaseModel]]]] = [
    (betterlite_models.MCPPublicServer, litellm_mcp.MCPPublicServer),
    (betterlite_models.MCPAllowedClient, litellm_mcp.MCPAllowedClient),
    (betterlite_models.MCPToolSearchSettings, litellm_mcp.MCPToolSearchSettings),
    (betterlite_hooks.MCPPreCallRequestObject, litellm_mcp.MCPPreCallRequestObject),
    (betterlite_hooks.MCPPreCallResponseObject, litellm_mcp.MCPPreCallResponseObject),
    (betterlite_hooks.MCPDuringCallRequestObject, litellm_mcp.MCPDuringCallRequestObject),
    (betterlite_hooks.MCPDuringCallResponseObject, litellm_mcp.MCPDuringCallResponseObject),
    (betterlite_hooks.MCPPostCallResponseObject, litellm_mcp.MCPPostCallResponseObject),
    (betterlite_hooks.MCPGatewaySession, litellm_mcp.MCPGatewaySession),
    (betterlite_hooks.MCPGatewaySessionGroupCount, litellm_mcp.MCPGatewaySessionGroupCount),
    (betterlite_hooks.MCPGatewaySessionsResponse, litellm_mcp.MCPGatewaySessionsResponse),
    (
        betterlite_hooks.MCPGatewaySessionsTerminateResponse,
        litellm_mcp.MCPGatewaySessionsTerminateResponse,
    ),
    (betterlite_manager.MCPOAuthMetadata, litellm_manager.MCPOAuthMetadata),
    (betterlite_manager.MCPOAuthIdentityBinding, litellm_manager.MCPOAuthIdentityBinding),
    (betterlite_manager.PinnedMCPTool, litellm_manager.PinnedMCPTool),
    (betterlite_manager.MCPServer, litellm_manager.MCPServer),
    (betterlite_toolset.MCPToolset, litellm_toolset.MCPToolset),
    (betterlite_toolset.NewMCPToolsetRequest, litellm_toolset.NewMCPToolsetRequest),
    (betterlite_toolset.UpdateMCPToolsetRequest, litellm_toolset.UpdateMCPToolsetRequest),
    (betterlite_registry.MCPTool, litellm_registry.MCPTool),
    (betterlite_registry.ToolSchema, litellm_registry.ToolSchema),
    (betterlite_registry.ListToolsResponse, litellm_registry.ListToolsResponse),
    (betterlite_registry.CallToolRequest, litellm_registry.CallToolRequest),
    (betterlite_registry.ContentItem, litellm_registry.ContentItem),
]
# MCPTool holds a callable, which has no JSON schema
SCHEMA_PAIRS: Final = [pair for pair in MODEL_PAIRS if pair[0] is not betterlite_registry.MCPTool]

ALIAS_ADAPTERS: Final[list[tuple[TypeAdapter[JsonValue], TypeAdapter[JsonValue]]]] = [
    (TypeAdapter(betterlite_enums.MCPTransportType), TypeAdapter(litellm_mcp.MCPTransportType)),
    (TypeAdapter(betterlite_enums.MCPLegacyVersion), TypeAdapter(litellm_mcp.MCPLegacyVersion)),
    (
        TypeAdapter(betterlite_enums.MCPUpstreamProtocol),
        TypeAdapter(litellm_mcp.MCPUpstreamProtocol),
    ),
    (
        TypeAdapter(betterlite_enums.MCPAdvertisedVersions),
        TypeAdapter(litellm_mcp.MCPAdvertisedVersions),
    ),
    (TypeAdapter(betterlite_enums.MCPSpecVersionType), TypeAdapter(litellm_mcp.MCPSpecVersionType)),
    (TypeAdapter(betterlite_enums.MCPAuthType), TypeAdapter(litellm_mcp.MCPAuthType)),
    (
        TypeAdapter(betterlite_enums.MCPTokenEndpointAuthMethod),
        TypeAdapter(litellm_mcp.MCPTokenEndpointAuthMethod),
    ),
]
BASE_SERVER: Final[dict[str, JsonValue]] = {"server_id": "s", "name": "n", "transport": "sse"}
OPAQUE: Final = "opaque-value"
SERVER_AXES: Final[dict[str, list[JsonValue]]] = {
    "auth_type": [None, *[auth.name for auth in litellm_mcp.MCPAuth]],
    "oauth2_flow": [None, "client_credentials", "authorization_code"],
    "delegate_auth_to_upstream": [False, True],
    "per_server_oauth_discovery": [False, True],
    "extra_headers": [
        None,
        [],
        ["Authorization"],
        ["X-Foo"],
        ["x-api-key"],
        ["X-Foo", "Api-Key"],
        ["authorization"],
    ],
    "oauth_passthrough": [False, True],
    "dcr_bridge": [None, False, True],
}
MCP_INFOS: Final[list[JsonValue]] = [
    {"protocol_version": "auto"},
    {"protocol_version": "2024-11-05"},
    {"protocol_version": "2025-11-25"},
    {"protocol_version": "2026-07-28"},
    {"protocol_version": "bogus"},
    {"protocol_version": 5},
    {"protocol_version": None},
    {"other": 1},
    {},
]
PROTOCOL_VERSIONS: Final[list[JsonValue]] = ["auto", "2025-06-18", "bogus", None]
HEADER_NAMES: Final = [
    "Authorization",
    " X-Api-Key ",
    "",
    "   ",
    "bad header",
    "a:b",
    "ok_name~",
    "x\r\ny",
    "é",
    "!#$%&'*+-.^_`|~09aZ",
    "a\nb",
]
HEADER_MAPS: Final[list[dict[str, str] | None]] = [
    None,
    {},
    {"Authorization": "a"},
    {"authorization": "a", "X-Key": "b"},
    {"X-Key": "b", "AUTHORIZATION": "a"},
    {"x-key": "b", "X-KEY": "c"},
    {"X-Key": "b", "Y-Key": "c"},
]
LOOKUP_NAMES: Final = ["Authorization", "authorization", "AUTHORIZATION", "x-key", "missing", ""]
ORIGIN_PAIRS: Final = [
    ("http://a", "http://a/path"),
    ("http://a", "https://a"),
    ("http://a:80", "https://a:443"),
    ("http://a:8080", "https://a:443"),
    ("http://a", "https://a:444"),
    ("https://a", "http://a"),
    ("https://a", "https://a:443"),
    ("https://a:443", "https://a"),
    ("https://a", "https://b"),
    ("https://a", "https://a:8443"),
    ("http://a", "http://a:81"),
    ("HTTP://A", "http://a"),
    ("http://a", "ftp://a"),
    ("", ""),
    ("a", "b"),
    ("http://a", ""),
]
HOOK_ARGS: Final[list[tuple[str, str | None]]] = [
    ("", "X-Key"),
    ("http://a", None),
    ("http://a", ""),
    ("http://a", "Authorization"),
    ("http://a", "authorization"),
    ("http://a", "X-Key"),
    ("http://a", "x-key"),
]
HOOK_TARGETS: Final = ["http://a/x", "https://a/x", "http://b/x", "http://a:81/x", "https://a:81/x"]
PINNED_VALUES: Final[list[JsonValue]] = [
    None,
    "",
    "{}",
    '{"t": {"description": "d", "input_schema": {"a": 1}}}',
    '{"t": {"extra": 1}}',
    '{"t": {}}',
    "not json",
    "[]",
    "0",
    "null",
    '"x"',
    "[1]",
    {},
    {"t": {"description": "d"}},
    {"t": {"input_schema": 3}},
    {"t": {"bogus": 1}},
    {"t": None},
    [],
    0,
    False,
    True,
    1,
    "5",
]
BINDINGS: Final[list[JsonValue]] = [
    {"issuer": "i", "audiences": ["a"]},
    {"mode": "audit", "issuer": "i", "audiences": ["a"]},
    {"mode": "enforce", "issuer": "i", "audiences": ["a"], "caller_field": "user_id"},
    {"mode": "enforce", "issuer": "i", "audiences": []},
    {"mode": "bogus", "issuer": "i", "audiences": ["a"]},
    {"mode": "audit", "audiences": ["a"]},
    {"mode": "audit", "issuer": "i", "caller_field": "other", "audiences": ["a"]},
]
UPSTREAM_HEADERS: Final[list[JsonValue]] = [
    None,
    "",
    "  ",
    "X-Token",
    " X-Token ",
    "bad header",
    "a\r\nb",
    5,
]
URL_PAIRS: Final[list[tuple[JsonValue, JsonValue]]] = [
    (None, None),
    (None, "c"),
    ("", "c"),
    ("a", None),
    ("a", "c"),
    ("a", ""),
]


class ObjectSchema(BaseModel):
    properties: dict[str, JsonValue]
    required: list[str] = []


def object_schema(model: type[BaseModel]) -> ObjectSchema:
    raw: dict[str, JsonValue] = model.model_json_schema()
    return ObjectSchema.model_validate(raw)


def field_shape(model: type[BaseModel]) -> list[tuple[str, bool, str]]:
    shape: list[tuple[str, bool, str]] = []
    for name, info in model.model_fields.items():
        if info.is_required():
            shape.append((name, True, ""))
            continue
        default: JsonValue = info.get_default(call_default_factory=True)
        shape.append((name, False, repr(default)))
    return shape


def outcome(model: type[BaseModel], payload: dict[str, JsonValue]) -> JsonValue:
    try:
        built = model.model_validate(payload)
    except ValidationError as error:
        failures: list[JsonValue] = [
            [detail["type"], [str(part) for part in detail["loc"]], detail["msg"]]
            for detail in error.errors()
        ]
        return failures
    dumped: dict[str, JsonValue] = built.model_dump(mode="json")
    return dumped


def pinned_result(
    parse: Callable[[JsonValue], Mapping[str, BaseModel] | None], value: JsonValue
) -> JsonValue:
    try:
        tools = parse(value)
    except ValueError as error:
        return [type(error).__name__, str(error)]
    if tools is None:
        return None
    dumped: dict[str, JsonValue] = {name: tool.model_dump_json() for name, tool in tools.items()}
    return dumped


def assert_server_flags_match(payload: dict[str, JsonValue]) -> None:
    ours = betterlite_manager.MCPServer.model_validate(payload)
    theirs = litellm_manager.MCPServer.model_validate(payload)
    assert ours.effective_authorization_url == theirs.effective_authorization_url, payload
    assert ours.effective_token_url == theirs.effective_token_url, payload
    assert ours.effective_registration_url == theirs.effective_registration_url, payload
    assert ours.has_client_credentials == theirs.has_client_credentials, payload
    assert ours.needs_user_oauth_token == theirs.needs_user_oauth_token, payload
    assert ours.is_gateway_managed_oauth2 == theirs.is_gateway_managed_oauth2, payload
    assert ours.uses_per_server_oauth_relay == theirs.uses_per_server_oauth_relay, payload
    assert (
        ours.advertises_gateway_authorization_server
        == theirs.advertises_gateway_authorization_server
    ), payload
    assert ours.is_true_passthrough == theirs.is_true_passthrough, payload
    assert ours.is_oauth_delegate == theirs.is_oauth_delegate, payload
    assert ours.is_client_forwarded_token == theirs.is_client_forwarded_token, payload
    assert ours.is_dcr_bridge == theirs.is_dcr_bridge, payload
    assert ours.requires_per_user_auth == theirs.requires_per_user_auth, payload
    assert ours.is_oauth_passthrough == theirs.is_oauth_passthrough, payload


def test_names_exist_in_litellm_and_betterlite() -> None:
    betterlite_mcp_names: set[str] = set()
    for module in BETTERLITE_MCP_MODULES:
        betterlite_mcp_names.update(dir(module))
    assert LITELLM_MCP_NAMES <= set(dir(litellm_mcp))
    assert LITELLM_MCP_NAMES <= betterlite_mcp_names
    assert LITELLM_MANAGER_NAMES <= set(dir(litellm_manager))
    assert LITELLM_MANAGER_NAMES <= set(dir(betterlite_manager))
    assert LITELLM_TOOLSET_NAMES <= set(dir(litellm_toolset))
    assert LITELLM_TOOLSET_NAMES <= set(dir(betterlite_toolset))
    assert LITELLM_REGISTRY_NAMES <= set(dir(litellm_registry))
    assert LITELLM_REGISTRY_NAMES <= set(dir(betterlite_registry))


def test_names_constants() -> None:
    assert betterlite_enums.DEFAULT_SUBJECT_TOKEN_TYPE == litellm_mcp.DEFAULT_SUBJECT_TOKEN_TYPE
    assert betterlite_enums.MCP_LEGACY_VERSIONS == litellm_mcp.MCP_LEGACY_VERSIONS
    assert betterlite_headers.DEFAULT_CREDENTIAL_HEADER == litellm_mcp.DEFAULT_CREDENTIAL_HEADER
    assert (
        betterlite_credentials.MCP_ADMIN_CONFIG_CREDENTIAL_KEYS
        == litellm_mcp.MCP_ADMIN_CONFIG_CREDENTIAL_KEYS
    )


@pytest.mark.parametrize(("ours", "theirs"), ENUM_PAIRS)
def test_names_enum_members(ours: type[StrEnum], theirs: type[Enum]) -> None:
    assert [member.name for member in ours] == [member.name for member in theirs]
    for member in theirs:
        assert ours[member.name] == member


def test_names_literal_aliases() -> None:
    for ours, theirs in ALIAS_ADAPTERS:
        ours_schema: dict[str, JsonValue] = ours.json_schema()
        theirs_schema: dict[str, JsonValue] = theirs.json_schema()
        assert ours_schema == theirs_schema


def test_names_typeddict_keys() -> None:
    assert (
        betterlite_credentials.MCPCredentials.__required_keys__
        == litellm_mcp.MCPCredentials.__required_keys__
    )
    assert (
        betterlite_credentials.MCPCredentials.__optional_keys__
        == litellm_mcp.MCPCredentials.__optional_keys__
    )
    assert (
        betterlite_models.MCPServerCostInfo.__required_keys__
        == litellm_mcp.MCPServerCostInfo.__required_keys__
    )
    assert (
        betterlite_models.MCPServerCostInfo.__optional_keys__
        == litellm_mcp.MCPServerCostInfo.__optional_keys__
    )
    assert (
        betterlite_models.MCPStdioConfig.__required_keys__
        == litellm_mcp.MCPStdioConfig.__required_keys__
    )
    assert (
        betterlite_models.MCPStdioConfig.__optional_keys__
        == litellm_mcp.MCPStdioConfig.__optional_keys__
    )
    assert (
        betterlite_toolset.MCPToolsetTool.__required_keys__
        == litellm_toolset.MCPToolsetTool.__required_keys__
    )
    assert (
        betterlite_toolset.MCPToolsetTool.__optional_keys__
        == litellm_toolset.MCPToolsetTool.__optional_keys__
    )


@pytest.mark.parametrize(("ours", "theirs"), MODEL_PAIRS)
def test_names_model_fields(ours: type[BaseModel], theirs: type[BaseModel]) -> None:
    assert field_shape(ours) == field_shape(theirs)
    assert dict(ours.model_config) == dict(theirs.model_config)


@pytest.mark.parametrize(("ours", "theirs"), SCHEMA_PAIRS)
def test_names_model_schemas(ours: type[BaseModel], theirs: type[BaseModel]) -> None:
    ours_schema = object_schema(ours)
    theirs_schema = object_schema(theirs)
    assert sorted(ours_schema.required) == sorted(theirs_schema.required)
    assert {
        name: prop for name, prop in ours_schema.properties.items() if name not in NARROWED_FIELDS
    } == {
        name: prop for name, prop in theirs_schema.properties.items() if name not in NARROWED_FIELDS
    }


def test_names_registry_private_meta() -> None:
    assert betterlite_registry.ListToolsResponse.__private_attributes__.keys() == (
        litellm_registry.ListToolsResponse.__private_attributes__.keys()
    )


@pytest.mark.parametrize("raw", HEADER_NAMES)
def test_normalize_upstream_header_name_matches(raw: str) -> None:
    assert betterlite_headers.normalize_upstream_header_name(
        raw
    ) == litellm_mcp.normalize_upstream_header_name(raw)


@pytest.mark.parametrize("name", LOOKUP_NAMES)
@pytest.mark.parametrize("headers", HEADER_MAPS)
def test_header_lookups_match(headers: dict[str, str] | None, name: str) -> None:
    assert betterlite_headers.same_header(name, "x-KEY") == litellm_mcp.same_header(name, "x-KEY")
    assert betterlite_headers.has_header(headers, name) == litellm_mcp.has_header(headers, name)
    assert betterlite_headers.without_header(headers, name) == litellm_mcp.without_header(
        headers, name
    )


@pytest.mark.parametrize("headers", HEADER_MAPS)
def test_custom_credential_slot_matches(headers: dict[str, str] | None) -> None:
    assert betterlite_headers.custom_credential_slot(
        headers
    ) == litellm_mcp.custom_credential_slot(headers)


@pytest.mark.parametrize(("configured", "target"), ORIGIN_PAIRS)
def test_crosses_origin_matches(configured: str, target: str) -> None:
    assert betterlite_headers.crosses_origin(configured, target) == litellm_mcp.crosses_origin(
        configured, target
    )


def test_crosses_origin_rejects_bad_port_like_litellm() -> None:
    with pytest.raises(ValueError, match="Port out of range") as theirs:
        litellm_mcp.crosses_origin("http://a:99999", "http://a")
    with pytest.raises(ValueError, match="Port out of range") as ours:
        betterlite_headers.crosses_origin("http://a:99999", "http://a")
    assert str(ours.value) == str(theirs.value)


@pytest.mark.parametrize("target", HOOK_TARGETS)
@pytest.mark.parametrize(("configured", "slot"), HOOK_ARGS)
def test_credential_redirect_hook_matches(configured: str, slot: str | None, target: str) -> None:
    ours = betterlite_headers.credential_redirect_hook(configured, slot)
    theirs = litellm_mcp.credential_redirect_hook(configured, slot)
    assert (ours is None) == (theirs is None)
    if ours is None or theirs is None:
        return
    ours_request = httpx.Request("GET", target, headers={"X-Key": "v", "Authorization": "a"})
    theirs_request = httpx.Request("GET", target, headers={"X-Key": "v", "Authorization": "a"})
    asyncio.run(asyncio.wait_for(ours(ours_request), None))
    asyncio.run(asyncio.wait_for(theirs(theirs_request), None))
    assert ours_request.headers == theirs_request.headers


@pytest.mark.parametrize("mcp_info", MCP_INFOS)
@pytest.mark.parametrize("protocol_version", PROTOCOL_VERSIONS)
def test_server_protocol_version_matches(mcp_info: JsonValue, protocol_version: JsonValue) -> None:
    with_both: dict[str, JsonValue] = {
        **BASE_SERVER,
        "mcp_info": mcp_info,
        "protocol_version": protocol_version,
    }
    only_info: dict[str, JsonValue] = {**BASE_SERVER, "mcp_info": mcp_info}
    only_version: dict[str, JsonValue] = {**BASE_SERVER, "protocol_version": protocol_version}
    for payload in (with_both, only_info, only_version, BASE_SERVER):
        assert outcome(betterlite_manager.MCPServer, payload) == outcome(
            litellm_manager.MCPServer, payload
        )


@pytest.mark.parametrize("binding", BINDINGS)
@pytest.mark.parametrize("auth_type", [None, "none", "oauth2", "true_passthrough"])
@pytest.mark.parametrize("flow", [None, "client_credentials", "authorization_code"])
@pytest.mark.parametrize("delegate", [False, True])
def test_server_identity_binding_matches(
    binding: JsonValue, auth_type: JsonValue, flow: JsonValue, delegate: JsonValue
) -> None:
    payload: dict[str, JsonValue] = {
        **BASE_SERVER,
        "auth_type": auth_type,
        "oauth2_flow": flow,
        "delegate_auth_to_upstream": delegate,
        "oauth_identity_binding": binding,
    }
    assert outcome(betterlite_manager.MCPServer, payload) == outcome(
        litellm_manager.MCPServer, payload
    )


@pytest.mark.parametrize("header", UPSTREAM_HEADERS)
def test_server_upstream_token_header_matches(header: JsonValue) -> None:
    payload: dict[str, JsonValue] = {**BASE_SERVER, "upstream_token_header": header}
    assert outcome(betterlite_manager.MCPServer, payload) == outcome(
        litellm_manager.MCPServer, payload
    )


@pytest.mark.parametrize(("transport", "auth_type"), [("sse", "bogus"), ("bogus", "oauth2")])
def test_server_literal_rejections_match(transport: str, auth_type: str) -> None:
    payload: dict[str, JsonValue] = {**BASE_SERVER, "transport": transport, "auth_type": auth_type}
    assert outcome(betterlite_manager.MCPServer, payload) == outcome(
        litellm_manager.MCPServer, payload
    )


def test_server_required_fields_match() -> None:
    assert outcome(betterlite_manager.MCPServer, {}) == outcome(litellm_manager.MCPServer, {})


@pytest.mark.parametrize(("authorization_url", "configured"), URL_PAIRS)
def test_server_effective_urls_match(authorization_url: JsonValue, configured: JsonValue) -> None:
    payload: dict[str, JsonValue] = {
        **BASE_SERVER,
        "authorization_url": authorization_url,
        "token_url": authorization_url,
        "registration_url": authorization_url,
        "configured_authorization_url": configured,
        "configured_token_url": configured,
        "configured_registration_url": configured,
    }
    assert_server_flags_match(payload)


def test_server_flags_match_over_every_auth_combination() -> None:
    for combination in itertools.product(*SERVER_AXES.values()):
        payload: dict[str, JsonValue] = {
            **BASE_SERVER,
            **dict(zip(SERVER_AXES, combination, strict=True)),
        }
        assert_server_flags_match(payload)


def test_server_repr_hides_credentials_like_litellm() -> None:
    payload: dict[str, JsonValue] = {
        **BASE_SERVER,
        "auth_type": "api_key",
        "authentication_token": OPAQUE,
        "client_secret": OPAQUE,
    }
    ours = betterlite_manager.MCPServer.model_validate(payload)
    theirs = litellm_manager.MCPServer.model_validate(payload)
    assert OPAQUE not in repr(ours)
    assert OPAQUE not in str(ours)
    assert repr(ours) == repr(theirs)
    assert str(ours) == str(theirs)


@pytest.mark.parametrize("value", PINNED_VALUES)
def test_parse_pinned_tools_matches(value: JsonValue) -> None:
    assert pinned_result(betterlite_manager.parse_pinned_tools, value) == pinned_result(
        litellm_manager.parse_pinned_tools, value
    )


@pytest.mark.parametrize("info", MCP_INFOS)
def test_protocol_metadata_check_matches(info: JsonValue) -> None:
    payload: dict[str, JsonValue] = {**BASE_SERVER, "mcp_info": info}
    assert outcome(betterlite_manager.MCPServer, payload) == outcome(
        litellm_manager.MCPServer, payload
    )
