import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Literal, NotRequired, TypedDict

import litellm
import openai
from pydantic import BaseModel, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

UPSTREAM: Final = "{{upstream}}"
KEPT_HEADERS: Final[tuple[str, ...]] = ("authorization", "x-scope")
ENV_PREFIXES: Final[tuple[str, ...]] = ("GROQ_", "CLOUDFLARE_", "FIREWORKS_AI_")

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
EMBEDDING_ANSWER: Final[JsonValue] = {
    "object": "list",
    "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}],
    "model": "oracle",
    "usage": {"prompt_tokens": 2, "total_tokens": 2},
}


class Raised(BaseModel):
    status_code: int


class Message(TypedDict):
    role: str
    content: str


class Answer(TypedDict):
    status: int
    body: str


class Globals(TypedDict):
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    groq_key: NotRequired[str]
    cloudflare_api_key: NotRequired[str]


class OptionalParams(TypedDict):
    temperature: NotRequired[int]
    top_p: NotRequired[int]
    extra_body: NotRequired[dict[str, JsonValue]]


class GroqConfig(TypedDict):
    temperature: NotRequired[int]
    top_p: NotRequired[int]


class Case(TypedDict):
    op: Literal["chat", "embedding"]
    provider: Literal["fireworks_ai", "together_ai", "groq", "cloudflare"]
    model: str
    messages: NotRequired[list[Message]]
    input: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    stream: NotRequired[bool]
    optional_params: NotRequired[OptionalParams]
    groq_config: NotRequired[GroqConfig]
    reasoning: NotRequired[bool]
    env: NotRequired[dict[str, str]]
    globals: NotRequired[Globals]
    answer: NotRequired[Answer]


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
        lowered = {name.lower(): value for name, value in self.headers.items()}
        server.requests.append(
            {
                "url": server.base + self.path,
                "headers": {
                    name: lowered[name] for name in KEPT_HEADERS if name in lowered
                },
                "body": (
                    None
                    if not content
                    else TypeAdapter(dict[str, JsonValue]).validate_json(content)
                ),
            }
        )
        payload = server.answer["body"].encode()
        self.send_response(server.answer["status"])
        content_type = "text/event-stream" if server.streaming else "application/json"
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class AnswerServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, answer: Answer, *, streaming: bool) -> None:
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


def answered(case: Case) -> Answer:
    reported = case.get("answer")
    if reported is not None:
        return reported
    default = EMBEDDING_ANSWER if case["op"] == "embedding" else CHAT_ANSWER
    return {"status": 200, "body": json.dumps(default)}


def optional_params(case: Case) -> OptionalParams:
    given = case.get("optional_params")
    if given is None:
        return {}
    return given


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
        litellm.groq_key = global_values.get("groq_key")
        litellm.cloudflare_api_key = global_values.get("cloudflare_api_key")
    config = case.get("groq_config")
    if config is not None:
        litellm.GroqChatConfig(
            temperature=config.get("temperature"), top_p=config.get("top_p")
        )
    api_base = case.get("api_base")
    if api_base is not None:
        case["api_base"] = api_base.replace(UPSTREAM, server.base)


def completion(case: Case) -> None:
    litellm.completion(
        model=f"{case['provider']}/{case['model']}",
        messages=case.get("messages"),
        api_base=case.get("api_base"),
        api_key=case.get("api_key"),
        headers=case.get("headers"),
        stream=case.get("stream"),
        **optional_params(case),
    )


def embedding(case: Case) -> None:
    litellm.embedding(
        model=f"{case['provider']}/{case['model']}",
        input=case.get("input"),
        api_base=case.get("api_base"),
        api_key=case.get("api_key"),
    )


def first_request(requests: list[RequestRecord], endpoint: str) -> RequestRecord | None:
    for recorded in requests:
        if recorded["url"].endswith(endpoint):
            return recorded
    return None


def request_view(recorded: RequestRecord | None, upstream: str) -> JsonValue:
    if recorded is None:
        return None
    headers: dict[str, JsonValue] = {
        name: recorded["headers"][name]
        for name in KEPT_HEADERS
        if name in recorded["headers"]
    }
    return {"url": recorded["url"].replace(upstream, UPSTREAM), "headers": headers}


def params_view(recorded: RequestRecord | None) -> JsonValue:
    if recorded is None:
        return None
    body = recorded["body"]
    if body is None:
        return None
    params: dict[str, JsonValue] = {}
    for name, value in body.items():
        if name in ("model", "messages"):
            continue
        params[name] = value
    return params


def value(case: Case, server: AnswerServer) -> JsonValue:
    error: JsonValue = None
    try:
        if case["op"] == "embedding":
            embedding(case)
        else:
            completion(case)
    except openai.APIError as raised:
        error = {"status": Raised.model_validate(raised, from_attributes=True).status_code}
    if case["op"] == "embedding":
        recorded = first_request(server.requests, "/embeddings")
        return {"request": request_view(recorded, server.base)}
    recorded = first_request(server.requests, "/chat/completions")
    return {
        "request": request_view(recorded, server.base),
        "params": params_view(recorded),
        "error": error,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    server = AnswerServer(answered(case), streaming=case.get("stream") is True)
    try:
        prepared(case, server)
        return value(case, server)
    finally:
        server.close()
