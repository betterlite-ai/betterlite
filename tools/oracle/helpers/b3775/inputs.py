from typing import Literal, NotRequired, TypedDict

from pydantic import JsonValue


class Answer(TypedDict):
    kind: Literal["answer"]
    status: int
    body: JsonValue


class Timeout(TypedDict):
    kind: Literal["timeout"]


class Closed(TypedDict):
    kind: Literal["close"]


Reply = Answer | Timeout | Closed


class Seen(TypedDict):
    method: str
    path: str
    authorization: str | None


class Seed(TypedDict):
    call_id: str
    hashes: list[str]


class LoggingSpec(TypedDict):
    call_id: NotRequired[str]
    agentic_loop_params: NotRequired[dict[str, JsonValue]]


class ShouldRunInput(TypedDict):
    kind: Literal["should_run"]
    response: JsonValue
    tools: list[dict[str, JsonValue]] | None


class PlanInput(TypedDict):
    kind: Literal["plan"]
    api_key: str | None
    timeout: float | None
    seed: Seed
    messages: list[dict[str, JsonValue]]
    tools: dict[str, JsonValue]
    model: str
    response: JsonValue
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    logging: LoggingSpec | None
    replies: list[Reply]


class HiddenlayerInput(TypedDict):
    kind: Literal["hiddenlayer"]
    version: Literal[1, 2]
    saas: bool
    api_id: str | None
    api_key: str | None
    timeout: float | None
    env: dict[str, str]
    tokens: list[Reply]
    detection: list[Reply] | None


Input = ShouldRunInput | PlanInput | HiddenlayerInput
