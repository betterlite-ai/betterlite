import json
from collections.abc import Mapping
from typing import Annotated, ClassVar, Final, Literal

from pydantic import AfterValidator, BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from betterlite.types.mcp.enums import MCPUpstreamProtocol

UPSTREAM_PROTOCOL: Final[TypeAdapter[MCPUpstreamProtocol]] = TypeAdapter(MCPUpstreamProtocol)


def _validate_mcp_protocol_metadata(  # ast-grep-ignore: no-long-names
    value: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    if "protocol_version" in value:
        UPSTREAM_PROTOCOL.validate_python(value["protocol_version"])
    return value


MCPInfo = Annotated[
    dict[str, JsonValue],
    AfterValidator(_validate_mcp_protocol_metadata),  # ast-grep-ignore: no-long-names
]


class MCPOAuthMetadata(BaseModel):
    scopes: list[str] | None = None
    authorization_url: str | None = None
    token_url: str | None = None
    registration_url: str | None = None
    discovered_issuer: str | None = None
    from_origin_fallback: bool = False


class MCPOAuthIdentityBinding(BaseModel):
    mode: Literal["disabled", "audit", "enforce"] = "disabled"
    issuer: str
    jwks_url: str | None = None
    audiences: Annotated[list[str], Field(min_length=1)]
    principal_claim: str = "email"
    caller_field: Literal["user_email", "user_id"] = "user_email"
    require_email_verified: bool = True


class PinnedMCPTool(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    description: str = ""
    input_schema: dict[str, JsonValue] = Field(default_factory=dict)


_PINNED_TOOLS: Final[TypeAdapter[dict[str, PinnedMCPTool] | None]] = TypeAdapter(
    dict[str, PinnedMCPTool] | None
)


def parse_pinned_tools(
    value: JsonValue | Mapping[str, PinnedMCPTool],
) -> dict[str, PinnedMCPTool] | None:
    decoded: JsonValue | Mapping[str, PinnedMCPTool] = value
    match value:
        case str() if value:
            decoded = json.loads(value)
        case str() | bool() | int() | float() | list() | Mapping() | None:
            pass
    if not decoded:
        return None
    return _PINNED_TOOLS.validate_python(decoded)
