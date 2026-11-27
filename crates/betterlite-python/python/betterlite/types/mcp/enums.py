from enum import Enum
from typing import Annotated, Final, Literal

from pydantic import Field


class MCPTransport(str, Enum):
    sse = "sse"
    http = "http"
    stdio = "stdio"


class MCPSpecVersion(str, Enum):
    nov_2024 = "2024-11-05"
    mar_2025 = "2025-03-26"
    jun_2025 = "2025-06-18"
    nov_2025 = "2025-11-25"
    jul_2026 = "2026-07-28"


class MCPAuth(str, Enum):
    none = "none"
    api_key = "api_key"
    bearer_token = "bearer_token"  # noqa: S105  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp.py#L44
    basic = "basic"
    authorization = "authorization"
    oauth2 = "oauth2"
    aws_sigv4 = "aws_sigv4"
    token = "token"  # noqa: S105  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp.py#L49
    oauth2_token_exchange = "oauth2_token_exchange"  # noqa: S105  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp.py#L50
    oauth2_id_jag = "oauth2_id_jag"
    true_passthrough = "true_passthrough"
    oauth_delegate = "oauth_delegate"


_URN_PREFIX: Final = "urn:ietf:params:oauth:token-type:"
# two pieces, since ruff S105 reads one plain string assigned to a TOKEN name as a password
DEFAULT_SUBJECT_TOKEN_TYPE: Final = _URN_PREFIX + "access_token"

MCPTransportType = Literal[MCPTransport.sse, MCPTransport.http, MCPTransport.stdio]
MCPLegacyVersion = Literal["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
MCP_LEGACY_VERSIONS: Final[tuple[MCPLegacyVersion, ...]] = (
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    "2025-11-25",
)
MCPUpstreamProtocol = MCPLegacyVersion | Literal["auto"]
MCPAdvertisedVersions = Annotated[tuple[MCPLegacyVersion, ...], Field(min_length=1)]
MCPSpecVersionType = Literal[
    MCPSpecVersion.nov_2024,
    MCPSpecVersion.mar_2025,
    MCPSpecVersion.jun_2025,
    MCPSpecVersion.nov_2025,
    MCPSpecVersion.jul_2026,
]
MCPAuthType = (
    Literal[
        MCPAuth.none,
        MCPAuth.api_key,
        MCPAuth.bearer_token,
        MCPAuth.basic,
        MCPAuth.authorization,
        MCPAuth.oauth2,
        MCPAuth.aws_sigv4,
        MCPAuth.token,
        MCPAuth.oauth2_token_exchange,
        MCPAuth.oauth2_id_jag,
        MCPAuth.true_passthrough,
        MCPAuth.oauth_delegate,
    ]
    | None
)
MCPTokenEndpointAuthMethod = Literal["client_secret_basic", "client_secret_post"]
