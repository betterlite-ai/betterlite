import asyncio
import socketserver

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import serve

from .records import WIRE, Capture, Input, Operation, SendFailureError, checked, recorded
from .scripted import (
    ClientSink,
    Probe,
    ScriptedProvider,
    Upstream,
    WireBackend,
    WireLog,
)

SETTLE_SECONDS = 10.0
DEADLINE_SECONDS = 120.0


async def apply(probe: Probe, op: Operation) -> JsonValue:
    try:
        match op["kind"]:
            case "send":
                return {"ok": await probe.send_to_backend(op["text"])}
            case "event":
                event = WIRE.validate_json(op["text"])
                return {"ok": await probe.send_event_to_client(event, op["text"])}
            case "disable":
                await probe.disable()
                return {"ok": True}
            case "cache":
                probe.cache_setup(op["text"])
                return {"ok": True}
    except SendFailureError as error:
        return {"error": str(error)}
    except AttributeError:
        return {"error": "non-object"}


async def session(input_value: Input, upstream: Upstream, socket: ClientConnection) -> JsonValue:
    callbacks: list[CustomLogger] = []
    for document in input_value["guardrails"]:
        hooks = [GuardrailEventHooks(name) for name in document["runs_for"]]
        custom = CustomGuardrail(guardrail_name=None, event_hook=hooks, default_on=True)
        callbacks.append(custom if document["custom"] else CustomLogger())
    litellm.callbacks = [*callbacks]
    timeline: list[JsonValue] = []
    capture = Capture()
    provider_document = input_value["provider"]
    provider = (
        None
        if provider_document is None
        else ScriptedProvider(provider_document, timeline, capture)
    )
    backend = WireBackend(socket, WireLog(input_value["backend_failures"], capture), timeline)
    client = ClientSink(
        WireLog(input_value["client_failures"], capture), beta=input_value["client_wants_beta"]
    )
    probe = Probe(input_value, backend, client, provider)
    results: list[JsonValue] = []
    for op in input_value["operations"]:
        results.append(await apply(probe, op))
        await asyncio.wait_for(upstream.wait_for(backend.log.delivered), SETTLE_SECONDS)
    client_frames: list[JsonValue] = [
        recorded("text", text.encode("utf-8")) for text in client.log.sent
    ]
    payload: dict[str, JsonValue] = {
        "results": results,
        "timeline": timeline,
        "upstream": upstream.frames,
        "client": client_frames,
        "state": probe.facts(),
    }
    return WIRE.validate_python(payload)


async def exchange(
    input_value: Input, upstream: Upstream, server: socketserver.TCPServer
) -> JsonValue:
    url = f"ws://127.0.0.1:{server.server_address[1]}"
    async with serve(upstream, sock=server.socket), connect(url) as socket:
        return await session(input_value, upstream, socket)


async def bounded(
    input_value: Input, upstream: Upstream, server: socketserver.TCPServer
) -> JsonValue:
    call = exchange(input_value, upstream, server)
    return await asyncio.wait_for(call, DEADLINE_SECONDS)


def run(input_value: Input) -> JsonValue:
    checked(input_value)
    upstream = Upstream()
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as server:
        return asyncio.run(bounded(input_value, upstream, server))
