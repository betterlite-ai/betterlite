from typing import Literal, NotRequired, TypedDict
from unittest.mock import patch

import httpx
import litellm
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

KEPT_HEADERS = ("authorization", "content-type")
MODEL_INFO_PATH = "/raw/main/config.json"


class Reply(TypedDict):
    status: int
    body: str


class Message(TypedDict):
    role: Literal["user"]
    content: str


class Case(TypedDict):
    op: Literal["huggingface_chat", "huggingface_embedding", "huggingface_rerank", "replicate", "predibase"]
    model: str
    messages: list[Message]
    stream: NotRequired[bool]
    input_type: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    input: list[str]
    query: str
    env: dict[str, str]
    replies: list[Reply]


class Recorder:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5849
        if request.url.path.endswith(MODEL_INFO_PATH):
            return httpx.Response(404)
        body: JsonValue = None
        if request.content:
            body = TypeAdapter(JsonValue).validate_json(request.content)
        headers: dict[str, JsonValue] = {}
        for name, value in request.headers.items():
            if name in KEPT_HEADERS or name.startswith("x-"):
                headers[name] = value
        self.requests.append({"method": request.method, "url": str(request.url), "headers": headers, "body": body})
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"].encode())


def provider_call(case: Case) -> None:
    messages = case["messages"]
    api_base = case.get("api_base")
    api_key = case.get("api_key")
    match case["op"]:
        case "huggingface_chat":
            litellm.completion(
                model=f"huggingface/{case['model']}",
                messages=messages,
                api_base=api_base,
                api_key=api_key,
                stream=case.get("stream"),
            )
        case "huggingface_embedding":
            litellm.embedding(
                model=f"huggingface/{case['model']}",
                input=case["input"],
                input_type=case.get("input_type"),
                api_base=api_base,
                api_key=api_key,
            )
        case "replicate":
            litellm.completion(model=f"replicate/{case['model']}", messages=messages, api_base=api_base, api_key=api_key)
        case "huggingface_rerank":
            litellm.rerank(
                model=f"huggingface/{case['model']}",
                query=case["query"],
                documents=case["input"],
                api_base=api_base,
                api_key=api_key,
            )
        case "predibase":
            litellm.completion(
                model=f"predibase/{case['model']}",
                messages=messages,
                api_base=api_base,
                api_key=api_key,
            )


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    recorder = Recorder(case["replies"])
    error: JsonValue = None
    with patch.object(httpx.HTTPTransport, "handle_request", recorder.handle):
        try:
            provider_call(case)
        except (litellm.APIError, litellm.APIConnectionError, litellm.BadRequestError, litellm.RateLimitError) as raised:
            error = {"status": raised.status_code}
    return {"requests": recorder.requests, "error": error}
