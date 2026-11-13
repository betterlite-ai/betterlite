import asyncio
import os
from typing import TypedDict

from litellm.proxy.middleware.per_request_root_path_middleware import (
    PerRequestRootPathMiddleware,
    get_request_root_path,
    normalize_root_paths,
)
from starlette._utils import get_route_path
from starlette.types import Message, Scope


class Input(TypedDict):
    scalar_root: str
    root_paths: list[str]
    requests: list[str]


class Seen(TypedDict):
    root: str
    path: str


class Capture:
    def __init__(self) -> None:
        self.seen: list[Seen] = []

    async def __call__(self, scope: Scope, _receive: object, _send: object) -> None:
        # An exact-root request is redirected by Starlette to the slash form, which routes as "/".
        self.seen.append({"root": get_request_root_path(), "path": get_route_path(scope) or "/"})


async def receive() -> Message:
    return {"type": "http.request"}


async def send(_message: Message) -> None:
    return None


async def run_request(root_paths: list[str], scalar_root: str, target: str) -> Seen:
    capture = Capture()
    middleware = PerRequestRootPathMiddleware(capture, root_paths)
    scope: Scope = {"type": "http", "path": target.split("?", 1)[0], "root_path": scalar_root}
    await middleware(scope, receive, send)
    return capture.seen[0]


def run(input_value: Input) -> dict[str, object]:
    os.environ["SERVER_ROOT_PATH"] = input_value["scalar_root"]
    normalized = list(normalize_root_paths(input_value["root_paths"]))
    if not normalized:
        return {"normalized": normalized, "installed": False, "requests": []}
    requests = [
        asyncio.run(run_request(normalized, input_value["scalar_root"], target))
        for target in input_value["requests"]
    ]
    return {"normalized": normalized, "installed": True, "requests": requests}
