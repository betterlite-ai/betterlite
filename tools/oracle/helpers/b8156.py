import asyncio
import json
import re
from collections.abc import Awaitable, Callable, Sequence
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.ollama.completion import handler as ollama_handler
from litellm.llms.ollama.completion.transformation import (
    OllamaConfig,
    OllamaTextCompletionResponseIterator,
)
from litellm.types.utils import EmbeddingResponse, ModelResponse

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Object: TypeAlias = dict[str, Json]
NOW = 1_700_000_000
CONFIG_KEYS = (
    "mirostat",
    "mirostat_eta",
    "mirostat_tau",
    "num_ctx",
    "num_gqa",
    "num_gpu",
    "num_thread",
    "repeat_last_n",
    "repeat_penalty",
    "temperature",
    "seed",
    "stop",
    "tfs_z",
    "num_predict",
    "top_k",
    "top_p",
    "system",
    "template",
)
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


class Encoding(Protocol):
    def encode(self, text: str, /) -> Sequence[int]: ...


class Function(Protocol):
    name: str | None
    arguments: str | None


class ToolCall(Protocol):
    id: str | None
    type: str | None
    function: Function


class Message(Protocol):
    role: str
    content: str | None
    reasoning_content: str | None
    tool_calls: list[ToolCall] | None


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: Json
    completion_tokens_details: Json


class Choice(Protocol):
    message: Message
    finish_reason: str


class Completion(Protocol):
    model: str
    created: int
    choices: list[Choice]
    usage: Usage


class Delta(Protocol):
    content: str | None
    reasoning_content: str | None


class StreamChoice(Protocol):
    index: int
    finish_reason: str | None
    delta: Delta


class StreamChunk(Protocol):
    id: str
    object: str
    created: int
    model: str | None
    usage: Usage | None
    choices: list[StreamChoice]


class EmbeddingRecord(Protocol):
    object: str
    index: int
    embedding: list[float]


class Embeddings(Protocol):
    object: str
    model: str
    data: list[EmbeddingRecord]
    usage: Usage


class WordsEncoding:
    def encode(self, text: str, /, **kwargs: object) -> list[int]:
        del kwargs
        return [1] * len(text.split())


class FailingEncoding:
    def encode(self, text: str, /, **kwargs: object) -> list[int]:
        del kwargs, text
        raise ValueError("encode failed")


class FakeClient:
    def __init__(self, status_code: int, body: str) -> None:
        self.status_code = status_code
        self.body = body
        self.url: str | None = None
        self.payload: Json = None

    async def post(self, url: str, json: Object) -> httpx.Response:
        self.url = url
        self.payload = cast("Json", json)
        return httpx.Response(
            self.status_code,
            content=self.body.encode(),
            request=httpx.Request("POST", url),
        )


def text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def number(value: Json) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError("expected an integer")
    return value


def items(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def maybe_obj(value: Json) -> Object:
    return dict(obj({} if value is None else value))


def status(error: Exception) -> int | None:
    carried = getattr(error, "status_code", None)
    if isinstance(carried, int) and not isinstance(carried, bool):
        return carried
    return None


def config_instance(data: Object) -> OllamaConfig:
    for key in CONFIG_KEYS:
        setattr(OllamaConfig, key, None)
    return cast("Callable[..., OllamaConfig]", OllamaConfig)(**data)


def uuid_text(value: str | None) -> str | None:
    return None if value is None else UUID_RE.sub("<uuid>", value)


def normalize_arguments(arguments: str) -> Json:
    try:
        parsed = cast("Json", json.loads(arguments))
    except json.JSONDecodeError:
        return arguments
    return canonical(parsed)


def canonical(value: Json) -> Json:
    encoded = json.dumps(value, separators=(",", ":"), ensure_ascii=False, sort_keys=True)
    return cast("Json", encoded)


def calls_value(calls: list[ToolCall] | None) -> Json:
    if calls is None:
        return None
    return [
        {
            "id": uuid_text(call.id),
            "type": call.type,
            "name": call.function.name,
            "arguments": normalize_arguments(text(call.function.arguments or "")),
        }
        for call in calls
    ]


def block_usage(usage: Json) -> Json:
    if usage is None:
        return None
    block = obj(usage)
    return [block["prompt_tokens"], block["completion_tokens"], block["total_tokens"]]


def usage_value(usage: Usage | None) -> Json:
    if usage is None:
        return None
    return [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens]


def content_value(value: str | None) -> Json:
    if value is None:
        return None
    try:
        return cast("Json", json.loads(value))
    except json.JSONDecodeError:
        return value


def reply_value(result: Completion) -> Json:
    choice = result.choices[0]
    message = choice.message
    return cast(
        "Json",
        {
            "model": result.model,
            "created": result.created,
            "finish_reason": choice.finish_reason,
            "role": message.role,
            "content": content_value(message.content),
            "reasoning_content": cast("str | None", getattr(message, "reasoning_content", None)),
            "tool_calls": calls_value(
                cast("list[ToolCall] | None", getattr(message, "tool_calls", None))
            ),
            "usage": usage_value(result.usage),
        },
    )


def chunk_value(chunk: StreamChunk) -> Json:
    choices = [
        cast(
            "Json",
            {
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": cast("str | None", getattr(choice.delta, "content", None)),
                "reasoning_content": cast(
                    "str | None", getattr(choice.delta, "reasoning_content", None)
                ),
            },
        )
        for choice in chunk.choices
    ]
    return cast(
        "Json",
        {
            "id": uuid_text(chunk.id),
            "object": chunk.object,
            "created": chunk.created,
            "model": cast("str | None", getattr(chunk, "model", None)),
            "usage": usage_value(cast("Usage | None", getattr(chunk, "usage", None))),
            "choices": choices,
        },
    )


def embedding_value(result: Embeddings) -> Json:
    return cast(
        "Json",
        {
            "object": result.object,
            "model": result.model,
            "data": [
                {
                    "object": record.get("object"),
                    "index": record.get("index"),
                    "embedding": record.get("embedding"),
                }
                for record in [obj(cast("Json", item)) for item in result.data]
            ],
            "usage": [
                result.usage.prompt_tokens,
                result.usage.completion_tokens,
                result.usage.total_tokens,
            ],
            "prompt_tokens_details": getattr(result.usage, "prompt_tokens_details", None),
            "completion_tokens_details": getattr(
                result.usage, "completion_tokens_details", None
            ),
        },
    )


def params(input_value: Object) -> Json:
    config = config_instance(maybe_obj(input_value.get("config")))
    mapped: list[Json] = []
    for entry in [obj(item) for item in items(input_value.get("cases") or [])]:
        optional = maybe_obj(entry.get("optional"))
        try:
            options = cast("Callable[..., Object]", config.map_openai_params)(
                non_default_params=maybe_obj(entry.get("non_default")),
                optional_params=optional,
                model=text(entry.get("model")),
                drop_params=False,
            )
            mapped.append(cast("Json", {"ok": True, "options": options}))
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            mapped.append(cast("Json", {"ok": False, "status": status(error)}))
    urls = [
        cast("Callable[..., str]", config.get_complete_url)(
            api_base=cast("str | None", url),
            api_key=None,
            model="model",
            optional_params={},
            litellm_params={},
        )
        for url in items(input_value.get("urls") or [])
    ]
    return cast(
        "Json",
        {
            "supported": cast("Callable[..., list[str]]", config.get_supported_openai_params)(
                model="model"
            ),
            "required": cast("Callable[..., list[Json]]", config.get_required_params)(),
            "config": cast("Callable[..., Object]", config.get_config)(),
            "mapped": mapped,
            "urls": urls,
        },
    )


def request(input_value: Object) -> Json:
    global_prompts = maybe_obj(input_value.get("custom_prompt_dict"))
    bodies: list[Json] = []
    with patch("litellm.custom_prompt_dict", new=global_prompts):
        for entry in [obj(item) for item in items(input_value.get("requests") or [])]:
            config = config_instance({})
            try:
                body = cast("Callable[..., Object]", config.transform_request)(
                    model=text(entry.get("model")),
                    messages=items(entry.get("messages")),
                    optional_params=maybe_obj(entry.get("options")),
                    litellm_params=maybe_obj(entry.get("litellm_params")),
                    headers={},
                )
                bodies.append(cast("Json", {"ok": True, "body": body}))
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                bodies.append(cast("Json", {"ok": False, "status": status(error)}))
    return cast("Json", {"bodies": bodies})


def reply(input_value: Object) -> Json:
    found: list[Json] = []
    with patch("litellm.encoding", new=WordsEncoding()), patch("time.time", return_value=NOW):
        for entry in [obj(item) for item in items(input_value.get("replies") or [])]:
            config = config_instance({})
            try:
                result = cast("Callable[..., ModelResponse]", config.transform_response)(
                    model=text(entry.get("model")),
                    raw_response=httpx.Response(
                        200,
                        content=text(entry.get("body")).encode(),
                        request=httpx.Request("POST", "https://host"),
                    ),
                    model_response=ModelResponse(),
                    logging_obj=None,
                    request_data=maybe_obj(entry.get("request_data")),
                    messages=[{"role": "user", "content": "hi"}],
                    optional_params={},
                    litellm_params={},
                    encoding=None,
                )
                found.append(
                    cast("Json", {"ok": True, "reply": reply_value(cast("Completion", result))})
                )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                found.append(cast("Json", {"ok": False, "status": status(error)}))
    return cast("Json", {"replies": found})


def stream(input_value: Object) -> Json:
    iterator = OllamaTextCompletionResponseIterator(
        streaming_response=iter([]), sync_stream=True, json_mode=False
    )
    handle = cast(
        "Callable[[str], StreamChunk | Object]",
        iterator._handle_string_chunk,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    )
    chunks: list[Json] = []
    with patch("time.time", return_value=NOW):
        for line in [text(item) for item in items(input_value.get("lines") or [])]:
            try:
                chunk = handle(line)
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                chunks.append(cast("Json", {"ok": False, "status": status(error)}))
                continue
            if isinstance(chunk, dict):
                found = obj(cast("Json", chunk))
                chunks.append(
                    cast(
                        "Json",
                        {
                            "ok": True,
                            "done": True,
                            "text": found.get("text"),
                            "is_finished": found.get("is_finished"),
                            "finish_reason": found.get("finish_reason"),
                            "usage": block_usage(found.get("usage")),
                        },
                    )
                )
            else:
                chunks.append(cast("Json", {"ok": True, "chunk": chunk_value(chunk)}))
    return cast("Json", {"chunks": chunks})


async def embedding_case(entry: Object) -> Json:
    client = FakeClient(number(entry.get("status")), text(entry.get("body")))
    encoder = {
        "none": None,
        "words": WordsEncoding(),
        "failing": FailingEncoding(),
    }[text(entry.get("encoder") or "none")]
    with patch("litellm.module_level_aclient", new=client):
        try:
            # LiteLLM types optional_params loosely, so the call site is only partially typed
            aembeddings = cast(
                "Callable[..., Awaitable[EmbeddingResponse]]",
                ollama_handler.ollama_aembeddings,  # pyright: ignore[reportUnknownMemberType]
            )
            result = await aembeddings(
                api_base=text(entry.get("api_base")),
                model=text(entry.get("model")),
                prompts=[text(prompt) for prompt in items(entry.get("prompts"))],
                model_response=EmbeddingResponse(),
                optional_params=maybe_obj(entry.get("options")),
                logging_obj=None,
                encoding=cast("Encoding | None", encoder),
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return cast("Json", {"ok": False, "status": status(error)})
    return cast(
        "Json",
        {
            "ok": True,
            "url": client.url,
            "payload": client.payload,
            "reply": embedding_value(cast("Embeddings", result)),
        },
    )


def embedding(input_value: Object) -> Json:
    cases = [obj(item) for item in items(input_value.get("cases") or [])]
    return cast("Json", {"cases": asyncio.run(embedding_cases(cases))})


async def embedding_cases(cases: list[Object]) -> list[Json]:
    return [await embedding_case(entry) for entry in cases]


OPS: dict[str, Callable[[Object], Json]] = {
    "params": params,
    "request": request,
    "reply": reply,
    "stream": stream,
    "embedding": embedding,
}


def run(input_value: Object) -> Json:
    return OPS[text(input_value["op"])](input_value)
