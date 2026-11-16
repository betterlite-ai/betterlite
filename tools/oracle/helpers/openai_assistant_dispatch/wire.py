import httpx
import openai
from pydantic import JsonValue
from typing_extensions import TypedDict

from .types import ORG, JSON, Canned, Input


class ClientOptions(TypedDict):
    api_key: str | None
    base_url: str | None
    organization: str
    max_retries: int


class Wire:
    def __init__(self, replies: list[Canned]) -> None:
        self.replies = list(replies)
        self.seen: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        body: JsonValue = JSON.validate_json(request.content) if request.content else None
        headers = request.headers
        row: dict[str, JsonValue] = {
            "method": request.method,
            "url": str(request.url),
            "authorization": headers.get("authorization"),
            "openai-organization": headers.get("openai-organization"),
            "openai-beta": headers.get("openai-beta"),
            "content-type": headers.get("content-type"),
            "body": body,
        }
        self.seen.append(row)
        canned = self.replies.pop(0)
        text = canned.get("text")
        content = text.encode("utf-8") if text is not None else JSON.dump_json(canned.get("body"))
        content_type = canned.get("content_type")
        sent = {} if content_type is None else {"content-type": content_type}
        return httpx.Response(canned["status"], content=content, headers=sent)


def base_options(input_value: Input) -> ClientOptions:
    return {
        "api_key": input_value["api_key"],
        "base_url": input_value["api_base"],
        "organization": ORG,
        "max_retries": 0,
    }


def sync_client(input_value: Input, wire: Wire) -> openai.OpenAI:
    transport = httpx.MockTransport(wire.handle)
    return openai.OpenAI(**base_options(input_value), http_client=httpx.Client(transport=transport))


def async_client(input_value: Input, wire: Wire) -> openai.AsyncOpenAI:
    transport = httpx.MockTransport(wire.handle)
    return openai.AsyncOpenAI(**base_options(input_value), http_client=httpx.AsyncClient(transport=transport))
