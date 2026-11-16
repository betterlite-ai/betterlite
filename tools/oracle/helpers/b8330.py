import os
import threading
from collections.abc import Iterable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Literal, NotRequired, TypedDict

import httpx
import litellm
import openai
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

UPSTREAM = "https://workspace.example.com"
PLACEHOLDER = "{{upstream}}"
TOKEN_BODY = b'{"access_token":"oracle-token"}'
REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

KEPT_HEADERS: dict[str, tuple[str, ...]] = {
    "databricks": ("authorization", "content-type"),
    "snowflake": (
        "authorization",
        "content-type",
        "accept",
        "x-snowflake-authorization-token-type",
        "anthropic-version",
    ),
    "oci": ("content-type", "user-agent"),
}
RECORDED_HEADERS = ("authorization", "content-type", "user-agent")


class Raised(BaseModel):
    status_code: int


class EmbeddingShape(BaseModel):
    model: str
    data: list[JsonValue]


class Message(TypedDict):
    role: Literal["user"]
    content: str


class Answer(TypedDict):
    status: int
    body: str


class TokenRequest(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    body: str


class RequestRecord(TypedDict):
    url: str
    headers: dict[str, JsonValue]
    body: JsonValue


class Case(TypedDict):
    op: Literal["chat", "embedding", "oci_stream"]
    provider: Literal["databricks", "snowflake", "oci"]
    model: str
    messages: NotRequired[list[Message]]
    input: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    stream: NotRequired[bool]
    optional_params: NotRequired[dict[str, JsonValue]]
    env: NotRequired[dict[str, str]]
    answer: NotRequired[Answer]


def kept(headers: httpx.Headers, names: Iterable[str]) -> dict[str, JsonValue]:
    return {name: headers[name] for name in names if name in headers}


def parsed(content: bytes) -> JsonValue:
    if not content:
        return None
    return REPLY.validate_json(content)


class Recorder:
    def __init__(self, answer: Answer, streaming: bool) -> None:
        self.answer = answer
        self.streaming = streaming
        self.requests: list[RequestRecord] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(
            {
                "url": str(request.url),
                "headers": kept(request.headers, RECORDED_HEADERS),
                "body": parsed(request.content),
            }
        )
        content_type = "text/event-stream" if self.streaming else "application/json"
        return httpx.Response(
            self.answer["status"],
            content=self.answer["body"].encode(),
            headers={"content-type": content_type},
        )

    def request_view(self, names: Iterable[str], upstream: str) -> JsonValue:
        if not self.requests:
            return None
        recorded = self.requests[0]
        return {
            "url": recorded["url"].replace(upstream, UPSTREAM),
            "headers": {name: recorded["headers"][name] for name in names if name in recorded["headers"]},
        }

    def embedding_view(self, names: Iterable[str], upstream: str) -> JsonValue:
        view = self.request_view(names, upstream)
        if view is None:
            return None
        return {
            "url": self.requests[0]["url"].replace(upstream, UPSTREAM),
            "headers": {name: self.requests[0]["headers"][name] for name in names if name in self.requests[0]["headers"]},
            "body": self.requests[0]["body"],
        }


def start_token_server(recorded: list[TokenRequest]) -> ThreadingHTTPServer:
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            length = self.headers.get("Content-Length")
            body = b"" if length is None else self.rfile.read(int(length))
            content_type = self.headers.get("Content-Type")
            authorization = self.headers.get("Authorization")
            recorded.append(
                {
                    "method": "POST",
                    "url": f"{UPSTREAM}{self.path}",
                    "headers": {
                        "content-type": "" if content_type is None else content_type,
                        "authorization": "" if authorization is None else authorization,
                    },
                    "body": body.decode(),
                }
            )
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(TOKEN_BODY)))
            self.end_headers()
            self.wfile.write(TOKEN_BODY)

    return ThreadingHTTPServer(("127.0.0.1", 0), Handler)


def substitute_environment(upstream: str) -> None:
    for name, value in list(os.environ.items()):
        if PLACEHOLDER in value:
            os.environ[name] = value.replace(PLACEHOLDER, upstream)


def call_provider(case: Case, client: HTTPHandler) -> JsonValue:
    model = f"{case['provider']}/{case['model']}"
    api_base = case.get("api_base")
    api_key = case.get("api_key")
    if case["op"] == "embedding":
        response = litellm.embedding(
            model=model,
            input=case["input"],
            api_base=api_base,
            api_key=api_key,
            client=client,
        )
        shape = EmbeddingShape.model_validate(response, from_attributes=True)
        return {"model": shape.model, "count": len(shape.data)}
    completion = litellm.completion(
        model=model,
        messages=case["messages"],
        api_base=api_base,
        api_key=api_key,
        stream=case.get("stream"),
        client=client,
    )
    if case["op"] == "oci_stream":
        if isinstance(completion, ModelResponse):
            return {"content": ""}
        text = ""
        for chunk in completion:
            content = chunk.choices[0].delta.content
            if content is not None:
                text += content
        return {"content": text}
    return None


def error_view(raised: openai.APIError) -> JsonValue:
    return {"status": Raised.model_validate(raised, from_attributes=True).status_code}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    recorded_tokens: list[TokenRequest] = []
    server = start_token_server(recorded_tokens)
    upstream = f"http://127.0.0.1:{server.server_address[1]}"
    threading.Thread(target=server.serve_forever, daemon=True).start()
    substitute_environment(upstream)
    api_base = case.get("api_base")
    if isinstance(api_base, str):
        case["api_base"] = api_base.replace(PLACEHOLDER, upstream)
    answer = case.get("answer")
    if answer is None:
        answer = {"status": 200, "body": ""}
    recorder = Recorder(answer, case.get("stream") is True)
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle)))
    litellm.module_level_client = client
    error: JsonValue = None
    reply: JsonValue = None
    try:
        reply = call_provider(case, client)
    except openai.APIError as raised:
        error = error_view(raised)
    names = KEPT_HEADERS[case["provider"]]
    if case["op"] == "chat":
        token_request: JsonValue = recorded_tokens[0] if recorded_tokens else None
        return {
            "request": recorder.request_view(names, upstream),
            "token_request": token_request,
            "error": error,
        }
    if case["op"] == "oci_stream":
        return {
            "request": recorder.request_view(names, upstream),
            "reply": reply if error is None else None,
            "error": error,
        }
    return {"request": recorder.embedding_view(names, upstream), "reply": reply, "error": error}
