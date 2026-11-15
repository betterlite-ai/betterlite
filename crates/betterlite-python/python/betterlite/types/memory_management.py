from datetime import datetime

from pydantic import BaseModel, Field, JsonValue


class _MemoryRow(BaseModel):
    memory_id: str
    key: str
    value: str
    metadata: JsonValue = None
    user_id: str | None = None
    team_id: str | None = None
    created_at: datetime | None = None
    created_by: str | None = None
    updated_at: datetime | None = None
    updated_by: str | None = None


LiteLLM_MemoryRow = _MemoryRow


class MemoryCreateRequest(BaseModel):
    key: str = Field(..., description="Memory key (acts as the namespace in the URL).")
    value: str = Field(
        ...,
        description="Memory content. Typically markdown/text for LLM context.",
    )
    metadata: JsonValue = Field(
        default=None,
        description="Optional JSON metadata (tags, structured fields).",
    )
    user_id: str | None = Field(
        default=None,
        description="Scope to this user. Defaults to the caller's user_id.",
    )
    team_id: str | None = Field(
        default=None,
        description="Scope to this team. Defaults to the caller's team_id.",
    )


class MemoryUpdateRequest(BaseModel):
    value: str | None = None
    metadata: JsonValue = None
    user_id: str | None = None
    team_id: str | None = None


class MemoryListResponse(BaseModel):
    memories: list[LiteLLM_MemoryRow]
    total: int


class MemoryDeleteResponse(BaseModel):
    key: str
    deleted: bool
