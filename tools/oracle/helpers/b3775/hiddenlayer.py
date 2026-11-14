import asyncio
import json
import os
import socket
import socketserver
import threading
import time
from collections import deque
from functools import partial
from typing import Final

import httpx
import requests
from litellm.proxy.guardrails.guardrail_hooks.hiddenlayer.hiddenlayer import (
    HiddenlayerGuardrail,
    HiddenlayerGuardrailV2,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from b3775.inputs import Answer, HiddenlayerInput, Reply, Seen
from b3775.service import Applier

JSON: Final = TypeAdapter(JsonValue)
SAAS_BASE: Final = "https://api.hiddenlayer.ai"
TOKEN_PREFIX: Final = "/oauth2/token"
INVALID_RESPONSE_PREFIX: Final = (
    "Unable to get authentication credentials for the HiddenLayer API - invalid response: "
)
TIMEOUT_SECONDS: Final = 0.3


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
            queue = self.tokens if path.startswith(TOKEN_PREFIX) else self.detection
            if queue is None:
                missing: Answer = {"kind": "answer", "status": 500, "body": {}}
                return missing
            if not queue:
                exhausted: Answer = {"kind": "answer", "status": 500, "body": {}}
                return exhausted
            return queue.popleft()


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
        length = self.headers.get("content-length")
        if length is not None and length.isdigit():
            self.rfile.read(int(length))
        seen: Seen = {
            "method": parts[0],
            "path": parts[1],
            "authorization": self.headers.get("authorization"),
        }
        self.store.record(seen)
        reply = self.store.admit(seen["path"])
        if reply["kind"] == "close":
            return
        if reply["kind"] == "timeout":
            time.sleep(TIMEOUT_SECONDS)
            reply = {"kind": "answer", "status": 200, "body": {"access_token": "late"}}
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
    matched = [entry for entry in seen if entry["path"].startswith(TOKEN_PREFIX) == tokens]
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
    if input_value["version"] == 2:
        return HiddenlayerGuardrailV2(
            api_id=input_value["api_id"],
            api_key=input_value["api_key"],
            api_base=api_base,
            auth_url=base,
            timeout=input_value["timeout"],
        )
    return HiddenlayerGuardrail(
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
    return agent, None


async def detect(agent: Applier) -> JsonValue:
    try:
        await agent.apply_guardrail(
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
    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), partial(Handler, store)) as server:
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{server.server_address[1]}"
        agent, raised = construct(input_value, base)
        if agent is not None and input_value["detection"] is not None:
            raised = asyncio.run(detect(agent))
        seen = store.calls()
        return JSON.validate_python(
            {
                "raised": raised,
                "token_requests": described(seen, tokens=True),
                "detection_requests": described(seen, tokens=False),
            }
        )
