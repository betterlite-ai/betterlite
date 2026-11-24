import asyncio
from datetime import datetime
from typing import Final, TypedDict

import litellm
import websockets.exceptions
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.responses.streaming_iterator import ResponsesWebSocketStreaming
from litellm.types.responses.streaming_websocket import (
    ResponsesBackendWebSocket,
    ResponsesClientWebSocket,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# exception mapping prints a feedback banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
STARTED: Final = datetime.fromisoformat("2026-01-01T00:00:00+00:00")


class Scenario(TypedDict):
    first_message: str | None
    client_frames: list[str]
    backend_frames: list[str]
    request_data: dict[str, JsonValue]


class Socket(ResponsesClientWebSocket, ResponsesBackendWebSocket):
    def __init__(self, frames: list[str]) -> None:
        self.sent: list[str] = []
        self.frames = frames

    @override
    async def send_text(self, data: str) -> None:
        await asyncio.sleep(0)
        self.sent.append(data)

    @override
    async def receive_text(self) -> str:
        await asyncio.sleep(0)
        return self.pop()

    @override
    async def recv(self, decode: bool = True) -> str | bytes:
        await asyncio.sleep(0)
        frame = self.pop()
        return frame if decode else frame.encode()

    @override
    async def send(self, message: str) -> None:
        await self.send_text(message)

    @override
    async def close(self) -> None:
        await asyncio.sleep(0)

    def pop(self) -> str:
        if not self.frames:
            raise websockets.exceptions.ConnectionClosed(None, None)
        return self.frames.pop(0)


async def run_scenario(scenario: Scenario) -> JsonValue:
    client = Socket(scenario["client_frames"])
    backend = Socket(scenario["backend_frames"])
    logging_obj = Logging("gpt-4o", None, stream=True, call_type="responses", start_time=STARTED,
                          litellm_call_id="call-1", function_id="fn-1")
    relay = ResponsesWebSocketStreaming(
        websocket=client, backend_ws=backend, logging_obj=logging_obj,
        request_data=TypeAdapter(dict[str, object]).validate_python(scenario["request_data"]),
        first_message=scenario["first_message"],
    )
    failure_returned = await relay.bidirectional_forward() is not None
    answer = {
        "sent_to_client": client.sent, "sent_to_backend": backend.sent,
        "messages": relay.messages, "input_messages": relay.input_messages,
        "litellm_metadata": relay.litellm_metadata, "failure_returned": failure_returned,
    }
    return JSON.validate_python(answer)


def run(argument: Scenario) -> JsonValue:
    with asyncio.Runner() as runner:
        return runner.run(run_scenario(argument))
