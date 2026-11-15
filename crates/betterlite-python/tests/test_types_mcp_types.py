import asyncio
import importlib
import itertools
from collections.abc import Callable, Mapping
from enum import Enum, StrEnum
from typing import Final, Protocol

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
import betterlite.types.mcp_server.server_metadata as betterlite_metadata
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
        "inputSchema",
        "input_schema",
        "mcp_info",
        "mcp_tool_call_response",
        "modified_arguments",
        "params",
        "token_validation",
        "user_api_key_auth",
    }
)

class ModelClasses(Protocol):
    def __getattr__(self, name: str) -> type[BaseModel]: ...


class ServerFlags(Protocol):
    @property
    def effective_authorization_url(self) -> str | None: ...

    @property
    def effective_token_url(self) -> str | None: ...

    @property
    def effective_registration_url(self) -> str | None: ...

    @property
    def has_client_credentials(self) -> bool: ...

    @property
    def needs_user_oauth_token(self) -> bool: ...

    @property
    def is_gateway_managed_oauth2(self) -> bool: ...

    @property
    def uses_per_server_oauth_relay(self) -> bool: ...

    @property
    def advertises_gateway_authorization_server(self) -> bool: ...  # ast-grep-ignore: no-long-names

    @property
    def is_true_passthrough(self) -> bool: ...

    @property
    def is_oauth_delegate(self) -> bool: ...

    @property
    def is_client_forwarded_token(self) -> bool: ...

    @property
    def is_dcr_bridge(self) -> bool: ...

    @property
    def requires_per_user_auth(self) -> bool: ...

    @property
    def is_oauth_passthrough(self) -> bool: ...


class ServerFlagsClass(Protocol):
    def model_validate(self, obj: JsonValue) -> ServerFlags: ...


class ServerFlagsClasses(Protocol):
    def __getattr__(self, name: str) -> ServerFlagsClass: ...


their_mcp: ModelClasses = importlib.import_module("litellm.types.mcp")
their_manager: ModelClasses = importlib.import_module("litellm.types.mcp_server.mcp_server_manager")
their_toolset: ModelClasses = importlib.import_module("litellm.types.mcp_server.mcp_toolset")
their_registry: ModelClasses = importlib.import_module("litellm.types.mcp_server.tool_registry")
their_flags: ServerFlagsClasses = importlib.import_module(
    "litellm.types.mcp_server.mcp_server_manager"
)

ENUM_PAIRS: Final[list[tuple[type[StrEnum], type[Enum]]]] = [
    (betterlite_enums.MCPTransport, litellm_mcp.MCPTransport),
    (betterlite_enums.MCPSpecVersion, litellm_mcp.MCPSpecVersion),
    (betterlite_enums.MCPAuth, litellm_mcp.MCPAuth),
]
BETTERLITE_MODELS: Final[list[type[BaseModel]]] = [
    betterlite_models.MCPPublicServer,
    betterlite_models.MCPAllowedClient,
    betterlite_models.MCPToolSearchSettings,
    betterlite_hooks.MCPPreCallRequestObject,
    betterlite_hooks.MCPPreCallResponseObject,
    betterlite_hooks.MCPDuringCallRequestObject,
    betterlite_hooks.MCPDuringCallResponseObject,
    betterlite_hooks.MCPPostCallResponseObject,
    betterlite_hooks.MCPGatewaySession,
    betterlite_hooks.MCPGatewaySessionGroupCount,
    betterlite_hooks.MCPGatewaySessionsResponse,
    betterlite_hooks.MCPGatewaySessionsTerminateResponse,
    betterlite_metadata.MCPOAuthMetadata,
    betterlite_metadata.MCPOAuthIdentityBinding,
    betterlite_metadata.PinnedMCPTool,
    betterlite_manager.MCPServer,
    betterlite_toolset.MCPToolset,
    betterlite_toolset.NewMCPToolsetRequest,
    betterlite_toolset.UpdateMCPToolsetRequest,
    betterlite_registry.MCPTool,
    betterlite_registry.ToolSchema,
    betterlite_registry.ListToolsResponse,
    betterlite_registry.CallToolRequest,
    betterlite_registry.ContentItem,
]
LITELLM_MODELS: Final[dict[str, type[BaseModel]]] = {
    "MCPPublicServer": their_mcp.MCPPublicServer,
    "MCPAllowedClient": their_mcp.MCPAllowedClient,
    "MCPToolSearchSettings": their_mcp.MCPToolSearchSettings,
    "MCPPreCallRequestObject": their_mcp.MCPPreCallRequestObject,
    "MCPPreCallResponseObject": their_mcp.MCPPreCallResponseObject,
    "MCPDuringCallRequestObject": their_mcp.MCPDuringCallRequestObject,
    "MCPDuringCallResponseObject": their_mcp.MCPDuringCallResponseObject,
    "MCPPostCallResponseObject": their_mcp.MCPPostCallResponseObject,
    "MCPGatewaySession": their_mcp.MCPGatewaySession,
    "MCPGatewaySessionGroupCount": their_mcp.MCPGatewaySessionGroupCount,
    "MCPGatewaySessionsResponse": their_mcp.MCPGatewaySessionsResponse,
    "MCPGatewaySessionsTerminateResponse": their_mcp.MCPGatewaySessionsTerminateResponse,
    "MCPOAuthMetadata": their_manager.MCPOAuthMetadata,
    "MCPOAuthIdentityBinding": their_manager.MCPOAuthIdentityBinding,
    "PinnedMCPTool": their_manager.PinnedMCPTool,
    "MCPServer": their_manager.MCPServer,
    "MCPToolset": their_toolset.MCPToolset,
    "NewMCPToolsetRequest": their_toolset.NewMCPToolsetRequest,
    "UpdateMCPToolsetRequest": their_toolset.UpdateMCPToolsetRequest,
    "MCPTool": their_registry.MCPTool,
    "ToolSchema": their_registry.ToolSchema,
    "ListToolsResponse": their_registry.ListToolsResponse,
    "CallToolRequest": their_registry.CallToolRequest,
    "ContentItem": their_registry.ContentItem,
}
THEIR_SERVER: Final = LITELLM_MODELS["MCPServer"]
# MCPTool holds a callable, which has no JSON schema
SCHEMA_MODELS: Final = [
    model for model in BETTERLITE_MODELS if model is not betterlite_registry.MCPTool
]

ALIAS_ADAPTERS: Final[list[tuple[TypeAdapter[JsonValue], TypeAdapter[JsonValue]]]] = [
    (TypeAdapter(betterlite_enums.MCPTransportType), TypeAdapter(litellm_mcp.MCPTransportType)),
    (TypeAdapter(betterlite_enums.MCPLegacyVersion), TypeAdapter(litellm_mcp.MCPLegacyVersion)),
    (
        TypeAdapter(betterlite_enums.MCPUpstreamProtocol),
        TypeAdapter(litellm_mcp.MCPUpstreamProtocol),
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
AUTH_TYPES: Final[list[JsonValue]] = [None, "none", "oauth2", "true_passthrough"]
FLOWS: Final[list[JsonValue]] = [None, "client_credentials", "authorization_code"]
FLAGS: Final[list[JsonValue]] = [False, True]
VERSION_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    payload
    for info, version in itertools.product(MCP_INFOS, PROTOCOL_VERSIONS)
    for payload in (
        {**BASE_SERVER, "mcp_info": info, "protocol_version": version},
        {**BASE_SERVER, "mcp_info": info},
        {**BASE_SERVER, "protocol_version": version},
        BASE_SERVER,
    )
]
BINDING_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    {
        **BASE_SERVER,
        "auth_type": auth_type,
        "oauth2_flow": flow,
        "delegate_auth_to_upstream": delegate,
        "oauth_identity_binding": binding,
    }
    for binding, auth_type, flow, delegate in itertools.product(BINDINGS, AUTH_TYPES, FLOWS, FLAGS)
]
TOKEN_HEADER_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    {**BASE_SERVER, "upstream_token_header": header} for header in UPSTREAM_HEADERS
]
INFO_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    {**BASE_SERVER, "mcp_info": info} for info in MCP_INFOS
]
REJECTED_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    {},
    {**BASE_SERVER, "transport": "sse", "auth_type": "bogus"},
    {**BASE_SERVER, "transport": "bogus", "auth_type": "oauth2"},
]
URL_PAYLOADS: Final[list[dict[str, JsonValue]]] = [
    {
        **BASE_SERVER,
        "authorization_url": primary,
        "token_url": primary,
        "registration_url": primary,
        "configured_authorization_url": configured,
        "configured_token_url": configured,
        "configured_registration_url": configured,
    }
    for primary, configured in URL_PAIRS
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
        return error.json(include_url=False, include_input=False)
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


def server_flags(server: ServerFlags) -> tuple[str | bool | None, ...]:
    return (
        server.effective_authorization_url,
        server.effective_token_url,
        server.effective_registration_url,
        server.has_client_credentials,
        server.needs_user_oauth_token,
        server.is_gateway_managed_oauth2,
        server.uses_per_server_oauth_relay,
        server.advertises_gateway_authorization_server,
        server.is_true_passthrough,
        server.is_oauth_delegate,
        server.is_client_forwarded_token,
        server.is_dcr_bridge,
        server.requires_per_user_auth,
        server.is_oauth_passthrough,
    )


def assert_server_flags_match(payload: dict[str, JsonValue]) -> None:
    ours = server_flags(betterlite_manager.MCPServer.model_validate(payload))
    theirs = server_flags(their_flags.MCPServer.model_validate(payload))
    assert ours == theirs, payload


def assert_server_outcomes_match(payload: dict[str, JsonValue]) -> None:
    ours = outcome(betterlite_manager.MCPServer, payload)
    theirs = outcome(THEIR_SERVER, payload)
    assert ours == theirs, payload


def test_names_exist() -> None:
    betterlite_mcp_names: set[str] = set()
    for module in BETTERLITE_MCP_MODULES:
        betterlite_mcp_names.update(dir(module))
    assert set(dir(litellm_mcp)) >= LITELLM_MCP_NAMES
    assert betterlite_mcp_names >= LITELLM_MCP_NAMES
    assert set(dir(litellm_manager)) >= LITELLM_MANAGER_NAMES
    assert set(dir(betterlite_manager)) | set(dir(betterlite_metadata)) >= LITELLM_MANAGER_NAMES
    assert set(dir(litellm_toolset)) >= LITELLM_TOOLSET_NAMES
    assert set(dir(betterlite_toolset)) >= LITELLM_TOOLSET_NAMES
    assert set(dir(litellm_registry)) >= LITELLM_REGISTRY_NAMES
    assert set(dir(betterlite_registry)) >= LITELLM_REGISTRY_NAMES


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


def test_names_advertised_versions() -> None:
    ours: dict[str, JsonValue] = TypeAdapter(betterlite_enums.MCPAdvertisedVersions).json_schema()
    theirs: dict[str, JsonValue] = TypeAdapter(litellm_mcp.MCPAdvertisedVersions).json_schema()
    assert ours == theirs


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


@pytest.mark.parametrize("ours", BETTERLITE_MODELS)
def test_names_model_fields(ours: type[BaseModel]) -> None:
    theirs = LITELLM_MODELS[ours.__name__]
    assert field_shape(ours) == field_shape(theirs)
    assert dict(ours.model_config) == dict(theirs.model_config)


@pytest.mark.parametrize("ours", SCHEMA_MODELS)
def test_names_model_schemas(ours: type[BaseModel]) -> None:
    ours_schema = object_schema(ours)
    theirs_schema = object_schema(LITELLM_MODELS[ours.__name__])
    assert sorted(ours_schema.required) == sorted(theirs_schema.required)
    assert {
        name: prop for name, prop in ours_schema.properties.items() if name not in NARROWED_FIELDS
    } == {
        name: prop for name, prop in theirs_schema.properties.items() if name not in NARROWED_FIELDS
    }


def test_names_registry_meta() -> None:
    assert betterlite_registry.ListToolsResponse.__private_attributes__.keys() == (
        LITELLM_MODELS["ListToolsResponse"].__private_attributes__.keys()
    )


def echo_handler(**arguments: JsonValue) -> JsonValue:
    echoed: dict[str, JsonValue] = dict(arguments)
    return echoed


def test_names_tool_handler() -> None:
    tool = betterlite_registry.MCPTool(
        name="n", description="d", input_schema={}, handler=echo_handler
    )
    assert tool.handler(a=1) == {"a": 1}
    payload: dict[str, JsonValue] = {
        "name": "n",
        "description": "d",
        "input_schema": {},
        "handler": 3,
    }
    with pytest.raises(ValidationError):
        betterlite_registry.MCPTool.model_validate(payload)
    with pytest.raises(ValidationError):
        LITELLM_MODELS["MCPTool"].model_validate(payload)


@pytest.mark.parametrize("raw", HEADER_NAMES)
def test_names_normalize_header(raw: str) -> None:
    assert betterlite_headers.normalize_upstream_header_name(
        raw
    ) == litellm_mcp.normalize_upstream_header_name(raw)


@pytest.mark.parametrize("name", LOOKUP_NAMES)
@pytest.mark.parametrize("headers", HEADER_MAPS)
def test_names_header_lookups(headers: dict[str, str] | None, name: str) -> None:
    assert betterlite_headers.same_header(name, "x-KEY") == litellm_mcp.same_header(name, "x-KEY")
    assert betterlite_headers.has_header(headers, name) == litellm_mcp.has_header(headers, name)
    assert betterlite_headers.without_header(headers, name) == litellm_mcp.without_header(
        headers, name
    )


@pytest.mark.parametrize("headers", HEADER_MAPS)
def test_names_credential_slot(headers: dict[str, str] | None) -> None:
    assert betterlite_headers.custom_credential_slot(
        headers
    ) == litellm_mcp.custom_credential_slot(headers)


@pytest.mark.parametrize(("configured", "target"), ORIGIN_PAIRS)
def test_names_crosses_origin(configured: str, target: str) -> None:
    assert betterlite_headers.crosses_origin(configured, target) == litellm_mcp.crosses_origin(
        configured, target
    )


def test_names_origin_bad_port() -> None:
    with pytest.raises(ValueError, match="Port out of range") as theirs:
        litellm_mcp.crosses_origin("http://a:99999", "http://a")
    with pytest.raises(ValueError, match="Port out of range") as ours:
        betterlite_headers.crosses_origin("http://a:99999", "http://a")
    assert str(ours.value) == str(theirs.value)


@pytest.mark.parametrize("target", HOOK_TARGETS)
@pytest.mark.parametrize(("configured", "slot"), HOOK_ARGS)
def test_names_redirect_hook(configured: str, slot: str | None, target: str) -> None:
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


def test_names_server_version() -> None:
    for payload in VERSION_PAYLOADS:
        assert_server_outcomes_match(payload)


def test_names_server_binding() -> None:
    for payload in BINDING_PAYLOADS:
        assert_server_outcomes_match(payload)


def test_names_token_header() -> None:
    for payload in TOKEN_HEADER_PAYLOADS:
        assert_server_outcomes_match(payload)


def test_names_server_rejections() -> None:
    for payload in REJECTED_PAYLOADS:
        assert_server_outcomes_match(payload)


def test_names_server_urls() -> None:
    for payload in URL_PAYLOADS:
        assert_server_flags_match(payload)


def test_names_server_flags() -> None:
    for combination in itertools.product(*SERVER_AXES.values()):
        payload: dict[str, JsonValue] = {
            **BASE_SERVER,
            **dict(zip(SERVER_AXES, combination, strict=True)),
        }
        assert_server_flags_match(payload)


def test_names_server_repr() -> None:
    payload: dict[str, JsonValue] = {
        **BASE_SERVER,
        "auth_type": "api_key",
        "authentication_token": OPAQUE,
        "client_secret": OPAQUE,
    }
    ours = betterlite_manager.MCPServer.model_validate(payload)
    theirs = THEIR_SERVER.model_validate(payload)
    assert OPAQUE not in repr(ours)
    assert OPAQUE not in str(ours)
    assert repr(ours) == repr(theirs)
    assert str(ours) == str(theirs)


@pytest.mark.parametrize("value", PINNED_VALUES)
def test_names_pinned_tools(value: JsonValue) -> None:
    ours = pinned_result(betterlite_metadata.parse_pinned_tools, value)
    theirs = pinned_result(litellm_manager.parse_pinned_tools, value)
    assert ours == theirs


def test_names_info_check() -> None:
    for payload in INFO_PAYLOADS:
        assert_server_outcomes_match(payload)
