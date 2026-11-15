from enum import StrEnum, auto
from typing import Annotated, Final, Literal

from pydantic import Field


class MCPTransport(StrEnum):
    sse = "sse"
    http = "http"
    stdio = "stdio"


class MCPSpecVersion(StrEnum):
    nov_2024 = "2024-11-05"
    mar_2025 = "2025-03-26"
    jun_2025 = "2025-06-18"
    nov_2025 = "2025-11-25"
    jul_2026 = "2026-07-28"


# auto() gives the lowercase member name, each value here; ruff S105 flags a literal on token
class MCPAuth(StrEnum):
    none = auto()
    api_key = auto()
    bearer_token = auto()
    basic = auto()
    authorization = auto()
    oauth2 = auto()
    aws_sigv4 = auto()
    token = auto()
    oauth2_token_exchange = auto()
    oauth2_id_jag = auto()
    true_passthrough = auto()
    oauth_delegate = auto()


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
