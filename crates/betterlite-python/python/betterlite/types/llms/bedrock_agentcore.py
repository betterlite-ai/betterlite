from typing import Literal

from typing_extensions import TypedDict


class AgentCoreRequestPayload(TypedDict):
    prompt: str


class AgentCoreRequest(TypedDict, total=False):
    payload: str


class AgentCoreMessageRole(TypedDict):
    role: Literal["assistant"]


class AgentCoreMessageStart(TypedDict):
    role: Literal["assistant"]


class AgentCoreContentBlockDelta(TypedDict):
    text: str


class AgentCoreContentBlockDeltaEvent(TypedDict):  # ast-grep-ignore: no-long-names
    delta: AgentCoreContentBlockDelta
    contentBlockIndex: int


class AgentCoreContentBlockStop(TypedDict):
    contentBlockIndex: int


class AgentCoreMessageStop(TypedDict):
    stopReason: Literal["end_turn", "max_tokens", "stop_sequence"]


class AgentCoreUsage(TypedDict):
    inputTokens: int
    outputTokens: int
    totalTokens: int


class AgentCoreMetrics(TypedDict):
    latencyMs: int


class AgentCoreMetadata(TypedDict):
    usage: AgentCoreUsage
    metrics: AgentCoreMetrics


class AgentCoreEventPayload(TypedDict, total=False):
    messageStart: AgentCoreMessageStart | None
    # ast-grep-ignore: no-long-names
    contentBlockDelta: AgentCoreContentBlockDeltaEvent | None
    contentBlockStop: AgentCoreContentBlockStop | None
    messageStop: AgentCoreMessageStop | None
    metadata: AgentCoreMetadata | None


class AgentCoreEvent(TypedDict, total=False):
    event: AgentCoreEventPayload | None


class AgentCoreContentBlock(TypedDict):
    text: str


class AgentCoreMessage(TypedDict):
    role: Literal["assistant"]
    content: list[AgentCoreContentBlock]


class AgentCoreFinalMessage(TypedDict):
    message: AgentCoreMessage


class AgentCoreParsedResponse(TypedDict):
    content: str
    usage: AgentCoreUsage | None
    final_message: AgentCoreMessage | None
