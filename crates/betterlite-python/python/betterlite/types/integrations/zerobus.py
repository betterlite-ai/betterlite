from dataclasses import dataclass, field
from typing import Final

from pydantic import Field

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams

RETRYABLE_INGEST_STATUS_CODES: Final = frozenset({408, 429, 500, 502, 503, 504})

TOKEN_REFRESH_LEEWAY_SECONDS: Final = 60


class ZerobusInitParams(StandardCustomLoggerInitParams):
    workspace_url: str | None = None
    server_endpoint: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    table_name: str | None = None
    batch_size: int = Field(default=100, gt=0)
    flush_interval: int = Field(default=10, gt=0)


@dataclass(frozen=True, slots=True)
class ZerobusConnection:
    workspace_url: str
    workspace_id: str
    server_endpoint: str
    client_id: str
    client_secret: str = field(repr=False)
    table_name: str


@dataclass(frozen=True, slots=True)
class ZerobusAccessToken:
    value: str = field(repr=False)
    expires_at: float


@dataclass(frozen=True, slots=True)
class ZerobusIngestFailure:
    detail: str
    retryable: bool
