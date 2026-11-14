import json
from collections import deque
from typing import Literal, NotRequired, Protocol, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.guardrails.guardrail_hooks.headroom.headroom import HeadroomGuardrail
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer.hiddenlayer import (
    HiddenlayerGuardrail,
    HiddenlayerGuardrailV2,
)
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
    hashes: list[JsonValue]


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


LiteLLMGuardrail = HeadroomGuardrail | HiddenlayerGuardrail | HiddenlayerGuardrailV2


class Guardrail(Protocol):
    async def apply_guardrail(
        self,
        inputs: dict[str, JsonValue],
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None,
    ) -> dict[str, JsonValue]: ...


class Applier:
    def __init__(self, guardrail: LiteLLMGuardrail) -> None:
        self.hook: Guardrail = cast(Guardrail, guardrail)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4536


class Service:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies: deque[Reply] = deque(replies)
        self.calls: list[Seen] = []

    def admit(self) -> Reply:
        if not self.replies:
            exhausted: Answer = {"kind": "answer", "status": 500, "body": {}}
            return exhausted
        return self.replies.popleft()

    def answer(self, reply: Answer) -> httpx.Response:
        encoded = json.dumps(reply["body"], separators=(",", ":")).encode()
        return httpx.Response(reply["status"], content=encoded)

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "method": request.method,
                "path": request.url.raw_path.decode(),
                "authorization": request.headers.get("authorization"),
            }
        )
        reply = self.admit()
        if reply["kind"] == "timeout":
            raise httpx.ReadTimeout("the headroom service timed out", request=request)
        if reply["kind"] == "close":
            raise httpx.ConnectError("the headroom service closed the connection", request=request)
        return self.answer(reply)
