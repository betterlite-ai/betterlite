from enum import StrEnum
from typing import Literal

from pydantic import BaseModel


class WeaveOtelConfig(BaseModel):
    otlp_auth_headers: str | None = None
    endpoint: str | None = None
    project_id: str | None = None
    protocol: Literal["otlp_grpc", "otlp_http"] = "otlp_http"


class WeaveSpanAttributes(StrEnum):
    DISPLAY_NAME = "wandb.display_name"
    THREAD_ID = "wandb.thread_id"
    IS_TURN = "wandb.is_turn"
