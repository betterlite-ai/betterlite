import asyncio
import json
import os
import socket
import socketserver
import threading
import time
from collections import deque
from collections.abc import Callable
from functools import partial
from io import BufferedIOBase
from typing import Final, TypeAlias

import httpx
import requests
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer.hiddenlayer import (
    HiddenlayerGuardrail,
    HiddenlayerGuardrailV2,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from b3775.service import Answer, Applier, HiddenlayerInput, Reply, Seen

JSON: Final = TypeAdapter(JsonValue)
SAAS_BASE: Final = "https://api.hiddenlayer.ai"
AUTH_PATH: Final = "/oauth2/token"
INVALID_RESPONSE_PREFIX: Final = (
    "Unable to get authentication credentials for the HiddenLayer API - invalid response: "
)
TIMEOUT_SECONDS: Final = 0.3
VERSION_TWO: Final = 2

# LiteLLM takes timeout only through **kwargs, which a typed Callable cannot name
Construct: TypeAlias = Callable[..., HiddenlayerGuardrail | HiddenlayerGuardrailV2]  # type: ignore[explicit-any]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/hiddenlayer/hiddenlayer.py#L229


class Store:
    def __init__(self, tokens: list[Reply], detection: list[Reply] | None) -> None:
        self.tokens: deque[Reply] = deque(tokens)
        self.detection: deque[Reply] | None = None if detection is None else deque(detection)
        self.seen: list[Seen] = []
        self.lock = threading.Lock()

    def record(self, seen: Seen) -> None:
        with self.lock:
            self.seen.append(seen)

    def calls(self) -> list[Seen]:
        with self.lock:
            return list(self.seen)

    def admit(self, path: str) -> Reply:
        with self.lock:
            queue = self.tokens if path.startswith(AUTH_PATH) else self.detection
            if queue is None or not queue:
                exhausted: Answer = {"kind": "answer", "status": 500, "body": {}}
                return exhausted
            return queue.popleft()


def read_headers(rfile: BufferedIOBase) -> dict[str, str]:
    headers: dict[str, str] = {}
    while True:
        line = rfile.readline().decode("latin-1")
        if not line or line in ("\r\n", "\n"):
            return headers
        name, _, value = line.partition(":")
        headers[name.strip().lower()] = value.strip()


class Handler(socketserver.StreamRequestHandler):
    def __init__(
        self,
        store: Store,
        request: socket.socket,
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
    ) -> None:
        self.store = store
        super().__init__(request, client_address, server)

    @override
    def handle(self) -> None:
        parts = self.rfile.readline().decode("latin-1").split(" ")
        headers = read_headers(self.rfile)
        length = headers.get("content-length")
        if length is not None and length.isdigit():
            self.rfile.read(int(length))
        seen: Seen = {
            "method": parts[0],
            "path": parts[1],
            "authorization": headers.get("authorization"),
        }
        self.store.record(seen)
        reply = self.store.admit(seen["path"])
        if reply["kind"] == "close":
            return
        if reply["kind"] == "timeout":
            time.sleep(TIMEOUT_SECONDS)
            self.answer({"kind": "answer", "status": 200, "body": {"access_token": "late"}})
            return
        self.answer(reply)

    def answer(self, reply: Answer) -> None:
        encoded = json.dumps(reply["body"], separators=(",", ":")).encode()
        head = (
            f"HTTP/1.1 {reply['status']} OK\r\n"
            "content-type: application/json\r\n"
            f"content-length: {len(encoded)}\r\n"
            "connection: close\r\n\r\n"
        )
        try:
            self.wfile.write(head.encode() + encoded)
        except OSError:
            return


def described(seen: list[Seen], *, tokens: bool) -> JsonValue:
    matched = [entry for entry in seen if entry["path"].startswith(AUTH_PATH) == tokens]
    return JSON.validate_python(matched)


def classify_runtime(message: str) -> JsonValue:
    if message.startswith("`api_id`"):
        return {"kind": "missing_api_id", "message": message}
    if message.startswith("`api_key`"):
        return {"kind": "missing_api_key", "message": message}
    if message.startswith(INVALID_RESPONSE_PREFIX):
        return {"kind": "invalid_response"}
    return {"kind": "token_status", "message": message}


def build(
    input_value: HiddenlayerInput, base: str
) -> HiddenlayerGuardrail | HiddenlayerGuardrailV2:
    api_base = SAAS_BASE if input_value["saas"] else base
    constructor: Construct = (
        HiddenlayerGuardrailV2
        if input_value["version"] == VERSION_TWO
        else HiddenlayerGuardrail
    )
    return constructor(
        api_id=input_value["api_id"],
        api_key=input_value["api_key"],
        api_base=api_base,
        auth_url=base,
        timeout=input_value["timeout"],
    )


def construct(input_value: HiddenlayerInput, base: str) -> tuple[Applier | None, JsonValue]:
    try:
        agent = build(input_value, base)
    except requests.Timeout:
        return None, {"kind": "timeout"}
    except requests.JSONDecodeError:
        return None, {"kind": "token_decode"}
    except requests.RequestException:
        return None, {"kind": "token_transport"}
    except RuntimeError as error:
        return None, classify_runtime(str(error))
    if input_value["saas"]:
        agent.api_base = base
    return Applier(agent), None


async def detect(agent: Applier) -> JsonValue:
    try:
        await agent.hook.apply_guardrail(
            {"structured_messages": [{"role": "user", "content": "hello"}]},
            {},
            "request",
            None,
        )
    except httpx.HTTPStatusError as error:
        return {"kind": "detection_status", "status": error.response.status_code}
    except httpx.TransportError:
        return {"kind": "detection_transport"}
    return None


def run(input_value: HiddenlayerInput) -> JsonValue:
    os.environ.update(input_value["env"])
    store = Store(input_value["tokens"], input_value["detection"])
    server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), partial(Handler, store))
    server.daemon_threads = True
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        base = f"http://127.0.0.1:{server.server_address[1]}"
        agent, raised = construct(input_value, base)
        if agent is not None and input_value["detection"] is not None:
            raised = asyncio.run(detect(agent))
        seen = store.calls()
        payload: dict[str, JsonValue] = {
            "raised": raised,
            "token_requests": described(seen, tokens=True),
            "detection_requests": described(seen, tokens=False),
        }
        return JSON.validate_python(payload)
    finally:
        server.shutdown()
        server.server_close()
