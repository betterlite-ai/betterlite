from typing import Annotated, ClassVar

from pydantic import BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import TypedDict

from betterlite.types.mcp.enums import MCPAuthType, MCPTransportType


class MCPPublicServer(BaseModel):
    server_id: str
    name: str
    alias: str | None = None
    server_name: str | None = None
    transport: MCPTransportType
    spec_path: str | None = None
    auth_type: MCPAuthType | None = None
    mcp_info: dict[str, JsonValue] | None = None


class MCPAllowedClient(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    alias: Annotated[
        str,
        Field(
            min_length=1,
            description=(
                "Human-readable name for this client application, "
                "shown in the dashboard and in gateway logs."
            ),
        ),
    ]
    value: Annotated[
        str,
        Field(
            min_length=1,
            description=(
                "Exact value of the JWT claim named in litellm_jwtauth.mcp_client_id_jwt_field, "
                "or of the mcp_client_id_header header, that identifies this client application. "
                "Matched case-sensitively."
            ),
        ),
    ]


class MCPToolSearchSettings(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    embedding_model: Annotated[
        str | None,
        Field(
            description=(
                "Embedding model from model_list used to rank tools by meaning. "
                "Unset keeps keyword matching."
            ),
        ),
    ] = None
    top_k: Annotated[
        int,
        Field(
            ge=1,
            le=100,
            description=(
                "Most ranked tools a search returns. A smaller top_k in the tool call wins. "
                "Core tools do not count."
            ),
        ),
    ] = 5
    similarity_threshold: Annotated[
        float,
        Field(
            ge=0.0,
            le=1.0,
            description=(
                "Lowest cosine similarity a tool needs to appear in semantic results "
                "(0.0 = no cutoff)."
            ),
        ),
    ] = 0.0
    core_tools: Annotated[
        tuple[str, ...],
        Field(
            description=(
                "Tool names always returned first when the caller can access them, "
                "e.g. `my_server-get_rates`."
            ),
        ),
    ] = ()


class MCPServerCostInfo(TypedDict, total=False):
    default_cost_per_query: float | None
    tool_name_to_cost_per_query: dict[str, float] | None


class MCPStdioConfig(TypedDict, total=False):
    command: str
    args: list[str]
    env: dict[str, str] | None
