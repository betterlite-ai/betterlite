from typing import Literal, NoReturn, TypedDict

from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.anthropic import (
    AppliedEdit,
    CompactionBlock,
    UsageIteration,
)
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json

from messages_sync_stream import Diagnostic, Initial, Message


class EmptyChunkError(Exception):
    pass


class FailureSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["reported", "empty"]
    status_code: int = 500
    message: str = ""
    after: int

    def raise_it(self) -> NoReturn:
        if self.kind == "reported":
            raise BaseLLMException(status_code=self.status_code, message=self.message)
        raise EmptyChunkError


class ChatRunSpec(BaseModel):
    model_config = ConfigDict(extra="forbid")

    initial: Initial = Field(default_factory=Initial)
    mode: Literal["wrapper", "sse", "view"]
    failure: FailureSpec | None = None
    steps: list[Literal["next", "drain", "close"]]


class MessagesChatStreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    messages: list[Message]
    api_key: str
    timeout_seconds: int
    tool_name_mapping: dict[str, str] = Field(default_factory=dict)
    applied_edits: list[AppliedEdit] = Field(default_factory=list)
    compaction_block: CompactionBlock | None = None
    iterations_usage: list[UsageIteration] | None = None
    runs: list[ChatRunSpec]


class FailureOutcome(TypedDict):
    kind: str
    status_code: int | None
    message: str


class EventOutcome(TypedDict):
    event: dict[str, Json] | None
    failure: FailureOutcome | None


class ViewOutcome(TypedDict):
    model: str
    chunks: list[str] | None


class StepOutcome(TypedDict):
    events: list[dict[str, Json]]
    frames: list[str]
    end: bool
    failure: FailureOutcome | None
    pulls: int
    diagnostics: list[Diagnostic]
    view: ViewOutcome | None


class RunOutcome(TypedDict):
    steps: list[StepOutcome]


class ChatValue(TypedDict):
    runs: list[RunOutcome]


class Outcome(TypedDict):
    messages_chat_stream: dict[Literal["value"], ChatValue]
