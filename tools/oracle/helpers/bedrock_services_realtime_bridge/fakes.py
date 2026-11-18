import asyncio
import sys
from types import ModuleType

from litellm.llms.bedrock.realtime.handler import BedrockPayloadPart
from pydantic import JsonValue

from .models import Chunk, CloseView, JsonAdapter, Run


class BedrockSendError(Exception): ...


class BedrockCloseError(Exception): ...


class BedrockAwaitError(Exception): ...


class BedrockReceiveError(Exception): ...


class ClientSendError(Exception): ...


class ClientGoneError(Exception): ...


class Counter:
    def __init__(self, threshold: int) -> None:
        self.threshold = threshold
        self.value = 0
        self.reached = asyncio.Event()

    def bump(self) -> int:
        index = self.value
        self.value += 1
        if self.value >= self.threshold:
            self.reached.set()
        return index


class FakePart:
    bytes_: bytes | None

    def __init__(self, bytes_: bytes | None) -> None:
        self.bytes_ = bytes_


class FakeChunk:
    value: BedrockPayloadPart | None

    def __init__(self, value: BedrockPayloadPart | None) -> None:
        self.value = value


def payload_text(event: FakeChunk) -> str:
    part = event.value
    if part is None:
        raise ValueError("the input chunk carries no payload part")
    payload = part.bytes_
    if payload is None:
        raise ValueError("the input payload part carries no bytes")
    return payload.decode("utf-8")


class FakeInput:
    def __init__(self, run_value: Run) -> None:
        self.failures = run_value["bedrock_send_failures"]
        self.close_error = run_value["bedrock_close_error"]
        self.sent: list[JsonValue] = []
        self.closed = False

    async def send(self, event: FakeChunk) -> None:
        index = len(self.sent)
        self.sent.append(JsonAdapter.validate_json(payload_text(event)))
        if index in self.failures:
            raise BedrockSendError("the input send failed")

    async def close(self) -> None:
        self.closed = True
        if self.close_error:
            raise BedrockCloseError("the input close failed")


def output_chunk(chunk: Chunk) -> FakeChunk | None:
    match chunk["kind"]:
        case "end":
            return None
        case "missing_value":
            return FakeChunk(None)
        case "missing_bytes":
            return FakeChunk(FakePart(None))
        case "empty":
            return FakeChunk(FakePart(b""))
        case "invalid_utf8":
            return FakeChunk(FakePart(b"\xff"))
        case "bytes":
            return FakeChunk(FakePart(chunk["text"].encode("utf-8")))


class FakeOutput:
    def __init__(self, run_value: Run) -> None:
        self.chunks = list(run_value["chunks"])
        self.failures = run_value["receive_failures"]
        self.block = run_value["block"]
        self.counter = Counter(len(self.chunks) + 1)

    async def receive(self) -> FakeChunk | None:
        index = self.counter.bump()
        if index in self.failures:
            raise BedrockReceiveError("the provider receive failed")
        if not self.chunks:
            if not self.block:
                return None
            await asyncio.Future[None]()
            return None
        return output_chunk(self.chunks.pop(0))


class FakeStream:
    def __init__(self, run_value: Run) -> None:
        self.input = FakeInput(run_value)
        self.output = FakeOutput(run_value)
        self.await_error = run_value["await_error"]

    @property
    def input_stream(self) -> FakeInput:
        return self.input

    async def await_output(self) -> tuple[JsonValue, FakeOutput]:
        if self.await_error is not None:
            raise BedrockAwaitError(self.await_error)
        return (None, self.output)


class FakeClient:
    def __init__(self, run_value: Run) -> None:
        self.scope: dict[str, JsonValue] = {}
        self.frames = list(run_value["frames"])
        self.failures = run_value["client_send_failures"]
        self.block = run_value["block"]
        self.counter = Counter(len(self.frames) + 1)
        self.sent: list[JsonValue] = []
        self.closed: CloseView | None = None

    async def receive_text(self) -> str:
        self.counter.bump()
        if self.frames:
            frame = self.frames.pop(0)
            if frame is not None:
                return frame
        if not self.block:
            raise ClientGoneError("the client is gone")
        await asyncio.Future[None]()
        raise ClientGoneError("the client is gone")

    async def send_text(self, data: str) -> None:
        index = len(self.sent)
        self.sent.append(JsonAdapter.validate_json(data))
        if index in self.failures:
            raise ClientSendError("the client send failed")

    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        if self.closed is None:
            self.closed = {"code": code, "reason": reason}


class FakeModels(ModuleType):
    def __init__(self) -> None:
        super().__init__("aws_sdk_bedrock_runtime.models")
        self.BidirectionalInputPayloadPart = FakePart
        self.InvokeModelWithBidirectionalStreamInputChunk = FakeChunk


class FakeRuntime(ModuleType):
    def __init__(self) -> None:
        super().__init__("aws_sdk_bedrock_runtime")


def install_sdk() -> None:
    sys.modules["aws_sdk_bedrock_runtime"] = FakeRuntime()
    sys.modules["aws_sdk_bedrock_runtime.models"] = FakeModels()
