import json
import re
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.ollama.completion.transformation import OllamaConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class ParamCase(TypedDict):
    model: str
    non_default: dict[str, Json]
    optional: NotRequired[dict[str, Json]]


class ParamsInput(TypedDict):
    op: Literal["params"]
    config: dict[str, Json]
    cases: list[ParamCase]
    urls: list[str | None]


class RequestCase(TypedDict):
    model: str
    messages: list[dict[str, Json]]
    options: NotRequired[dict[str, Json]]
    litellm_params: NotRequired[dict[str, Json]]


class RequestInput(TypedDict):
    op: Literal["request"]
    custom_prompt_dict: dict[str, dict[str, Json]]
    requests: list[RequestCase]


class ReplyCase(TypedDict):
    model: str
    body: str
    request_data: NotRequired[dict[str, Json]]


class ReplyInput(TypedDict):
    op: Literal["reply"]
    replies: list[ReplyCase]


class StreamInput(TypedDict):
    op: Literal["stream"]
    lines: list[str]


class EmbeddingCase(TypedDict):
    api_base: str
    model: str
    prompts: list[str]
    options: NotRequired[dict[str, Json]]
    status: int
    body: str
    encoder: NotRequired[str]


class EmbeddingInput(TypedDict):
    op: Literal["embedding"]
    cases: list[EmbeddingCase]


Input = ParamsInput | RequestInput | ReplyInput | StreamInput | EmbeddingInput

NOW = 1_700_000_000
UUID_PATTERN = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


class ConfigConstructor(Protocol):
    def __call__(self, **fields: Json) -> OllamaConfig: ...


def config_builder(data: dict[str, Json]) -> OllamaConfig:
    constructor = cast("ConfigConstructor", OllamaConfig)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L140
    return constructor(**data)


def dict_or_empty(value: dict[str, Json] | None) -> dict[str, Json]:
    if value is None:
        return {}
    return value


def content_value(value: str | None) -> Json:
    if value is None:
        return None
    try:
        return cast("Json", json.loads(value))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L305
    except json.JSONDecodeError:
        return value


def arguments_value(arguments: str) -> Json:
    try:
        parsed = cast("Json", json.loads(arguments))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L323
    except json.JSONDecodeError:
        return arguments
    return json.dumps(parsed, separators=(",", ":"), ensure_ascii=False, sort_keys=True)


class UsageRecord(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


def usage_list(usage: UsageRecord | None) -> Json:
    if usage is None:
        return None
    return [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens]


class FrozenClock:
    def __call__(self) -> float:
        return float(NOW)


class WordCounter:
    def encode(self, text: str, *, disallowed_special: tuple[()]) -> list[int]:
        del disallowed_special
        return [1] * len(text.split())


class FakeClient:
    def __init__(self, status_code: int, body: str) -> None:
        self.status_code = status_code
        self.body = body
        self.url: str | None = None
        self.payload: Json = None

    async def post(self, url: str, json: dict[str, Json]) -> httpx.Response:
        self.url = url
        self.payload = json
        return httpx.Response(
            self.status_code,
            content=self.body.encode(),
            request=httpx.Request("POST", url),
        )
