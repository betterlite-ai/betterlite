import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Literal, NotRequired, TypedDict

import litellm
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

UPSTREAM: Final = "{{upstream}}"
REFERER_HEADERS: Final[tuple[str, ...]] = (
    "HTTP-Referer",
    "X-Title",
    "http-referer",
    "x-title",
)
ENV_PREFIXES: Final[tuple[str, ...]] = ("OPENROUTER_", "OR_", "VERCEL_")

CHAT_ANSWER: Final[JsonValue] = {
    "id": "chatcmpl-1",
    "object": "chat.completion",
    "created": 1700000000,
    "model": "oracle",
    "service_tier": "default",
    "choices": [
        {
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop",
        }
    ],
    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
}
CHAT_STREAM: Final = (
    'data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1700000000,'
    '"model":"oracle","choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},'
    '"finish_reason":"stop"}]}\n\n'
    "data: [DONE]\n\n"
)
EMBEDDING_ANSWER: Final[JsonValue] = {
    "object": "list",
    "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}],
    "model": "oracle",
    "usage": {"prompt_tokens": 2, "total_tokens": 2},
}


class Message(TypedDict):
    role: str
    content: str


class Globals(TypedDict):
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    openrouter_key: NotRequired[str]


class OptionalParams(TypedDict):
    temperature: NotRequired[int]
    top_p: NotRequired[int]
    stop: NotRequired[list[str]]
    extra_body: NotRequired[dict[str, JsonValue]]


class Case(TypedDict):
    op: Literal["chat", "embedding"]
    provider: Literal["openrouter", "vercel_ai_gateway"]
    model: str
    messages: NotRequired[list[Message]]
    input: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    stream: NotRequired[bool]
    optional_params: NotRequired[OptionalParams]
    class_config: NotRequired[OptionalParams]
    env: NotRequired[dict[str, str]]
    globals: NotRequired[Globals]


class RequestRecord(TypedDict):
    url: str
    headers: dict[str, str]
    body: dict[str, JsonValue] | None


class AnswerHandler(BaseHTTPRequestHandler):
    server_ref: ClassVar["AnswerServer | None"] = None

    def do_POST(self) -> None:
        server = AnswerHandler.server_ref
        if server is None:
            raise RuntimeError("the handler is not on an AnswerServer")
        length = self.headers.get("Content-Length")
        content = b"" if length is None else self.rfile.read(int(length))
        server.requests.append(
            {
                "url": server.base + self.path,
                "headers": {name: value for name, value in self.headers.items()},
                "body": (
                    None
                    if not content
                    else TypeAdapter(dict[str, JsonValue]).validate_json(content)
                ),
            }
        )
        payload = server.answer.encode()
        self.send_response(200)
        self.send_header(
            "content-type",
            "text/event-stream" if server.streaming else "application/json",
        )
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class AnswerServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, answer: str, *, streaming: bool) -> None:
        super().__init__(("127.0.0.1", 0), AnswerHandler)
        self.answer = answer
        self.streaming = streaming
        self.requests: list[RequestRecord] = []
        AnswerHandler.server_ref = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


def prepared(case: Case, server: AnswerServer) -> None:
    for name in list(os.environ):
        if name.startswith(ENV_PREFIXES):
            del os.environ[name]
    environment = case.get("env")
    if environment is not None:
        for name, value in environment.items():
            os.environ[name] = value.replace(UPSTREAM, server.base)
    global_values = case.get("globals")
    if global_values is not None:
        global_base = global_values.get("api_base")
        litellm.api_base = (
            None if global_base is None else global_base.replace(UPSTREAM, server.base)
        )
        litellm.api_key = global_values.get("api_key")
        litellm.openrouter_key = global_values.get("openrouter_key")
    config = case.get("class_config")
    if config is not None and case["provider"] == "openrouter":
        litellm.OpenrouterConfig(
            temperature=config.get("temperature"),
            top_p=config.get("top_p"),
            stop=config.get("stop"),
        )
    elif config is not None:
        litellm.VercelAIGatewayConfig(
            temperature=config.get("temperature"),
            top_p=config.get("top_p"),
            stop=config.get("stop"),
        )
    api_base = case.get("api_base")
    if api_base is not None:
        case["api_base"] = api_base.replace(UPSTREAM, server.base)


def kept_headers(headers: dict[str, str]) -> dict[str, JsonValue]:
    kept: dict[str, JsonValue] = {}
    for name, value in headers.items():
        if name.lower() == "authorization":
            kept["authorization"] = value
        elif name in REFERER_HEADERS:
            kept[name] = value
    return kept


def request_view(recorded: RequestRecord | None, upstream: str) -> JsonValue:
    if recorded is None:
        return None
    return {
        "url": recorded["url"].replace(upstream, UPSTREAM),
        "headers": kept_headers(recorded["headers"]),
    }


def params_view(recorded: RequestRecord | None) -> JsonValue:
    if recorded is None or recorded["body"] is None:
        return None
    return {
        name: value
        for name, value in recorded["body"].items()
        if name not in ("model", "messages")
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    embedding = case["op"] == "embedding"
    default = EMBEDDING_ANSWER if embedding else CHAT_ANSWER
    optional = case.get("optional_params")
    if optional is None:
        optional = OptionalParams()
    streaming = case.get("stream") is True
    server = AnswerServer(
        CHAT_STREAM if streaming else json.dumps(default), streaming=streaming
    )
    try:
        prepared(case, server)
        if embedding:
            litellm.embedding(
                model=f"{case['provider']}/{case['model']}",
                input=case.get("input"),
                api_base=case.get("api_base"),
                api_key=case.get("api_key"),
            )
        else:
            litellm.completion(
                model=f"{case['provider']}/{case['model']}",
                messages=case.get("messages"),
                api_base=case.get("api_base"),
                api_key=case.get("api_key"),
                headers=case.get("headers"),
                stream=case.get("stream"),
                **optional,
            )
        endpoint = "/embeddings" if embedding else "/chat/completions"
        recorded = next(
            (item for item in server.requests if item["url"].endswith(endpoint)), None
        )
        view: dict[str, JsonValue] = {"request": request_view(recorded, server.base)}
        if not embedding:
            view["params"] = params_view(recorded)
        return view
    finally:
        server.close()
