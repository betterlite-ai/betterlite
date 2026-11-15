from dataclasses import dataclass
from typing import Final

from pydantic import Field

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams

RETRYABLE_UPLOAD_STATUS_CODES: Final = frozenset({429, 500, 502, 503, 504})

DEFAULT_API_URL: Final = "https://api.pointfive.co/api/v1/ingestion"


class PointFiveInitParams(StandardCustomLoggerInitParams):
    api_key: str | None = None
    api_url: str | None = None
    batch_size: int = Field(default=1_000, gt=0)
    flush_interval: int = Field(default=300, gt=0)
    max_batch_bytes: int = Field(default=8 * 1024 * 1024, gt=0)
    max_upload_retries: int = Field(default=3, ge=1)


@dataclass(frozen=True, slots=True)
class PointFiveUploadTarget:
    upload_url: str
    object_key: str


@dataclass(frozen=True, slots=True)
class PointFiveUploadFailure:
    detail: str
    retryable: bool
