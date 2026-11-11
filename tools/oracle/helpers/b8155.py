import json
import re
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.ollama.chat.transformation import (
    OllamaChatCompletionResponseIterator,
    OllamaChatConfig,
)
from litellm.types.utils import ModelResponse

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Object: TypeAlias = dict[str, Json]
NOW = 1_700_000_000
CONFIG = cast("Callable[..., OllamaChatConfig]", OllamaChatConfig)
CONFIG_KEYS = (
    "mirostat",
    "mirostat_eta",
    "mirostat_tau",
    "num_ctx",
    "num_gqa",
    "num_thread",
    "repeat_last_n",
    "repeat_penalty",
    "seed",
    "tfs_z",
    "num_predict",
    "top_k",
    "system",
    "template",
    "temperature",
    "stop",
    "top_p",
)
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


class Function(Protocol):
    name: str | None
    arguments: str | None


class ToolCall(Protocol):
    id: str | None
    type: str | None
    function: Function


class Delta(Protocol):
    content: str | None
    reasoning_content: str | None
    tool_calls: list[ToolCall] | None


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class StreamChoice(Protocol):
    index: int
    finish_reason: str | None
    delta: Delta


class StreamChunk(Protocol):
    id: str
    object: str
    created: int
    model: str
    choices: list[StreamChoice]
    usage: Usage | None


class Message(Protocol):
    role: str
    content: str | None
    reasoning_content: str | None
    tool_calls: list[ToolCall] | None


class Choice(Protocol):
    message: Message
    finish_reason: str


class Completion(Protocol):
    model: str
    created: int
    choices: list[Choice]
    usage: Usage


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def status_of(error: Exception) -> int | None:
    carried = getattr(error, "status_code", None)
    if isinstance(carried, int) and not isinstance(carried, bool):
        return carried
    return None


def config_instance(data: Object) -> OllamaChatConfig:
    for key in CONFIG_KEYS:
        setattr(OllamaChatConfig, key, None)
    return CONFIG(**data)


def plain_object(value: Json) -> Object:
    return dict(expect_object(value if value is not None else {}))


def normalize_id(value: str | None) -> str | None:
    if value is None:
        return None
    return UUID_RE.sub("<uuid>", value)


def normalize_arguments(value: Json) -> Json:
    if not isinstance(value, str):
        return value
    try:
        parsed = cast("Json", json.loads(value))
    except json.JSONDecodeError:
        return value
    return json.dumps(parsed, separators=(",", ":"), ensure_ascii=False, sort_keys=True)


def tool_calls_value(calls: list[ToolCall] | None) -> Json:
    if calls is None:
        return None
    return [
        {
            "id": normalize_id(call.id),
            "type": call.type,
            "name": call.function.name,
            "arguments": normalize_arguments(call.function.arguments),
        }
        for call in calls
    ]


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
            "content": message.content,
            "reasoning_content": cast(
                "str | None", getattr(message, "reasoning_content", None)
            ),
            "tool_calls": tool_calls_value(
                cast("list[ToolCall] | None", getattr(message, "tool_calls", None))
            ),
            "usage": [
                result.usage.prompt_tokens,
                result.usage.completion_tokens,
                result.usage.total_tokens,
            ],
        },
    )


def chunk_value(chunk: StreamChunk) -> Json:
    choices: list[Json] = []
    for choice in chunk.choices:
        delta = choice.delta
        choices.append(
            cast(
                "Json",
                {
                    "index": choice.index,
                    "finish_reason": choice.finish_reason,
                    "content": cast("str | None", getattr(delta, "content", None)),
                    "reasoning_content": cast(
                        "str | None", getattr(delta, "reasoning_content", None)
                    ),
                    "tool_calls": tool_calls_value(
                        cast(
                            "list[ToolCall] | None",
                            getattr(delta, "tool_calls", None),
                        )
                    ),
                },
            )
        )
    usage = chunk.usage
    return cast(
        "Json",
        {
            "id": normalize_id(chunk.id),
            "object": chunk.object,
            "created": chunk.created,
            "model": chunk.model,
            "usage": None
            if usage is None
            else [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
            "choices": choices,
        },
    )


def params(input_value: Object) -> Json:
    config = config_instance(plain_object(input_value.get("config")))
    mapped: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["cases"])]:
        optional = plain_object(entry.get("optional"))
        non_default = plain_object(entry["non_default"])
        try:
            options = cast("Callable[..., Object]", config.map_openai_params)(
                non_default_params=non_default,
                optional_params=optional,
                model=expect_text(entry["model"]),
                drop_params=False,
            )
            mapped.append(cast("Json", {"ok": True, "options": options}))
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            mapped.append(cast("Json", {"ok": False, "status": status_of(error)}))
    validated: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value.get("validate") or [])]:
        headers = plain_object(entry.get("headers"))
        try:
            found = cast("Callable[..., Object]", config.validate_environment)(
                headers=headers,
                model="model",
                messages=[],
                optional_params={},
                litellm_params={},
                api_key=cast("str | None", entry.get("api_key")),
            )
            validated.append(cast("Json", dict(expect_object(cast("Json", found)))))
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            validated.append(cast("Json", {"ok": False, "status": status_of(error)}))
    urls = [
        cast("Callable[..., str]", config.get_complete_url)(
            api_base=cast("str | None", url),
            api_key=None,
            model="model",
            optional_params={},
            litellm_params={},
        )
        for url in expect_list(input_value.get("urls") or [])
    ]
    supported = cast("Callable[..., list[str]]", config.get_supported_openai_params)(
        model="model"
    )
    configured = cast("Callable[..., Object]", config.get_config)()
    return cast(
        "Json",
        {
            "supported": supported,
            "config": configured,
            "mapped": mapped,
            "validated": validated,
            "urls": urls,
        },
    )


def request(input_value: Object) -> Json:
    bodies: list[Json] = []
    for entry in [expect_object(item) for item in expect_list(input_value["requests"])]:
        config = config_instance(plain_object(entry.get("config")))
        litellm_params: Object = {}
        try:
            body = cast("Callable[..., Object]", config.transform_request)(
                model=expect_text(entry["model"]),
                messages=expect_list(entry["messages"]),
                optional_params=plain_object(entry.get("options")),
                litellm_params=litellm_params,
                headers={},
            )
            bodies.append(
                cast(
                    "Json",
                    {
                        "ok": True,
                        "body": body,
                        "function_name": litellm_params.get("function_name"),
                    },
                )
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            bodies.append(cast("Json", {"ok": False, "status": status_of(error)}))
    return cast("Json", {"bodies": bodies})


def fake_token_counter(
    *, text: str | None = None, messages: list[Json] | None = None, **kwargs: object
) -> int:
    del kwargs
    if messages is not None:
        total = 0
        for message in messages:
            content = expect_object(message).get("content")
            total += len(str(content if content is not None else "").split())
        return total
    if text is not None:
        return len(text.split())
    return 0


class NullLogging:
    def post_call(self, **kwargs: object) -> None:
        del kwargs


def response(body: str) -> httpx.Response:
    return httpx.Response(
        200,
        content=body.encode(),
        request=httpx.Request("POST", "https://host"),
    )


def reply(input_value: Object) -> Json:
    found: list[Json] = []
    with (
        patch("litellm.token_counter", new=fake_token_counter),
        patch("time.time", return_value=NOW),
    ):
        for entry in [expect_object(item) for item in expect_list(input_value["replies"])]:
            config = config_instance({})
            try:
                result = cast("Callable[..., ModelResponse]", config.transform_response)(
                    model=expect_text(entry["model"]),
                    raw_response=response(expect_text(entry["body"])),
                    model_response=ModelResponse(),
                    logging_obj=NullLogging(),
                    request_data=plain_object(entry.get("request_data")),
                    messages=[{"role": "user", "content": "hi"}],
                    optional_params={},
                    litellm_params=plain_object(entry.get("litellm_params")),
                    encoding=None,
                )
                found.append(
                    cast(
                        "Json",
                        {"ok": True, "reply": reply_value(cast("Completion", result))},
                    )
                )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                found.append(cast("Json", {"ok": False, "status": status_of(error)}))
    return cast("Json", {"replies": found})


def stream(input_value: Object) -> Json:
    iterator = OllamaChatCompletionResponseIterator(
        streaming_response=iter([]), sync_stream=True, json_mode=False
    )
    handle = cast(
        "Callable[[str], StreamChunk | Object]",
        iterator._handle_string_chunk,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    )
    chunks: list[Json] = []
    with patch("time.time", return_value=NOW):
        for line in [expect_text(item) for item in expect_list(input_value["lines"])]:
            try:
                chunk = handle(line)
                if isinstance(chunk, dict):
                    finished = bool(chunk.get("is_finished"))
                    chunks.append(
                        cast("Json", {"ok": True, "done": True})
                        if finished
                        else cast("Json", {"ok": True, "skipped": True})
                    )
                else:
                    chunks.append(
                        cast(
                            "Json",
                            {"ok": True, "chunk": chunk_value(chunk)},
                        )
                    )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                chunks.append(cast("Json", {"ok": False, "status": status_of(error)}))
    return cast("Json", {"chunks": chunks})


OPS: dict[str, Callable[[Object], Json]] = {
    "params": params,
    "request": request,
    "reply": reply,
    "stream": stream,
}


def run(input_value: Object) -> Json:
    operation = expect_text(input_value["op"])
    return OPS[operation](input_value)
