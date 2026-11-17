import asyncio
import json
import threading
from collections.abc import Coroutine
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import SimpleNamespace, TracebackType
from typing import ClassVar, Final, Protocol, TypedDict, cast

from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    tinyfish_passthrough_logging_handler as tinyfish,
)
from pydantic import JsonValue
from typing_extensions import override

API_KEY_VARIABLE: Final = "TINYFISH_API_KEY"
API_BASE_VARIABLE: Final = "TINYFISH_AGENT_API_BASE"


class Reply(TypedDict):
    status: int
    body: JsonValue


class Request(TypedDict):
    path: str
    api_key: str | None


# the clock surface the poller reads off its module: monotonic, sleep and create_task
class Timer(Protocol):
    def monotonic(self) -> float: ...
    def create_task(self, coro: Coroutine[None, None, None]) -> asyncio.Task[None]: ...
    async def sleep(self, delay: float) -> None: ...


class Handlers(Protocol):
    time: SimpleNamespace
    asyncio: SimpleNamespace


class Script:
    def __init__(self, replies: list[Reply] | None) -> None:
        self.replies = [] if replies is None else list(replies)
        self.requests: list[Request] = []

    def answer(self) -> Reply:
        if len(self.replies) > 1:
            return self.replies.pop(0)
        return self.replies[0]

    def summary(self, api_base: str) -> dict[str, JsonValue]:
        paths: list[JsonValue] = []
        keys: list[JsonValue] = []
        for request in self.requests:
            path = request["path"].removeprefix(api_base)
            if path not in paths:
                paths.append(path)
            if request["api_key"] not in keys:
                keys.append(request["api_key"])
        return {"count": len(self.requests), "paths": paths, "api_keys": keys}


class Answering(BaseHTTPRequestHandler):
    script: ClassVar["Script | None"] = None

    def do_GET(self) -> None:
        script = Answering.script
        if script is None:
            raise RuntimeError("the handler is not on a scripted server")
        script.requests.append(
            {"path": self.path, "api_key": self.headers.get("X-API-Key")}
        )
        reply = script.answer()
        body = json.dumps(reply["body"]).encode()
        self.send_response(reply["status"])
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class Scripted(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, script: Script) -> None:
        super().__init__(("127.0.0.1", 0), Answering)
        self.script = script
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def start(self) -> None:
        self.thread.start()

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class Served:
    def __init__(self, script: Script) -> None:
        self.script = script
        self.server = Scripted(script)

    def __enter__(self) -> str:
        Answering.script = self.script
        self.server.start()
        return self.server.base

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.server.close()


class Clock(Timer):
    def __init__(self) -> None:
        self.now = 0.0
        self.tasks: list[asyncio.Task[None]] = []

    @override
    def monotonic(self) -> float:
        return self.now

    @override
    def create_task(self, coro: Coroutine[None, None, None]) -> asyncio.Task[None]:
        task = asyncio.ensure_future(coro)
        self.tasks.append(task)
        return task

    @override
    async def sleep(self, delay: float) -> None:
        self.now += delay
        await asyncio.sleep(0)


# the poller reads time and asyncio off its own module, so the fake clock goes there
def install(clock: Clock) -> None:
    handlers: Final[Handlers] = cast(Handlers, tinyfish)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/tinyfish_passthrough_logging_handler.py#L13-L14
    handlers.time = SimpleNamespace(monotonic=clock.monotonic)
    handlers.asyncio = SimpleNamespace(sleep=clock.sleep, create_task=clock.create_task)
