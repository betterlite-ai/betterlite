import json
from collections import deque
from typing import Literal, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue

from .inputs import Answer, Reply, Seen


class Applier(Protocol):
    async def apply_guardrail(
        self,
        inputs: dict[str, JsonValue],
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None,
    ) -> dict[str, JsonValue]: ...


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
