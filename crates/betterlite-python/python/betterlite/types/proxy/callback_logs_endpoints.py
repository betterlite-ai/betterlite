from typing import Final, Literal

from pydantic import BaseModel, Field, JsonValue

MAX_CALLBACK_LOG_RECORDS: Final = 1000


class CallbackLogRecord(BaseModel):
    status: Literal["success", "failure"]
    standard_logging_payload: dict[str, JsonValue]
    error: str | None = None


class CallbackLogsRequest(BaseModel):
    records: list[CallbackLogRecord] = Field(..., max_length=MAX_CALLBACK_LOG_RECORDS)


class CallbackLogFailure(BaseModel):
    index: int
    error: str


class CallbackLogsResponse(BaseModel):
    processed: int
    failed: int
    failures: list[CallbackLogFailure] = Field(default_factory=list)
