from collections.abc import Callable, Mapping
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.bytez.chat.transformation import (
    BytezChatConfig,
    BytezCustomStreamWrapper,
    adapt_messages_to_bytez_standard,  # pyright: ignore[reportUnknownVariableType]
    get_tokens_from_messages,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.utils import ModelResponse, ModelResponseStream, Usage

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging
    from litellm.types.llms.openai import AllMessageValues

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

CONFIG = BytezChatConfig()
MAP_PARAMS = cast("Callable[..., Object]", CONFIG.map_openai_params)
VALIDATE = cast("Callable[..., Object]", CONFIG.validate_environment)
COMPLETE_URL = cast("Callable[..., str]", CONFIG.get_complete_url)
TRANSFORM_REQUEST = cast("Callable[..., Object]", CONFIG.transform_request)
TRANSFORM_RESPONSE = cast("Callable[..., object]", CONFIG.transform_response)
ADAPT = cast(
    "Callable[[list[AllMessageValues]], list[dict[str, object]]]",
    adapt_messages_to_bytez_standard,
)
TOKENS = cast("Callable[[list[dict[str, object]]], int]", get_tokens_from_messages)


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def at(fields: Object, name: str) -> Object:
    return obj(fields.get(name, {}))


def messages(fields: Object) -> "list[AllMessageValues]":
    value = fields.get("messages", [])
    if not isinstance(value, list):
        raise TypeError("messages must be a list")
    return cast("list[AllMessageValues]", value)


def headers(fields: Object) -> dict[str, str]:
    return {
        name: value
        for name, value in at(fields, "headers").items()
        if isinstance(value, str)
    }


def options(fields: Object) -> Json:
    model = cast(str, fields.get("supported_model", ""))
    mapped: list[Json] = []
    for entry in cast("list[Json]", fields.get("mappings", [])):
        mapping = obj(entry)
        try:
            adapted = MAP_PARAMS(
                at(mapping, "non_default_params"),
                at(mapping, "optional_params"),
                model,
                bool(mapping.get("drop_params", False)),
            )
        except Exception:  # noqa: BLE001 - the status is what a caller can branch on
            mapped.append({"ok": False})
            continue
        mapped.append({"ok": True, "value": adapted})
    supported = cast(Json, list(CONFIG.get_supported_openai_params(model)))
    return {"supported": supported, "mapped": mapped}


def usage(fields: Object) -> Json:
    try:
        normalized = ADAPT(messages(fields))
        prompt_tokens = TOKENS(normalized)
    except Exception:  # noqa: BLE001 - the status is what a caller can branch on
        return {"ok": False, "status": None}
    return {
        "ok": True,
        "value": {"normalized": cast(Json, normalized), "prompt_tokens": prompt_tokens},
    }


def request_entry(entry: Json) -> Json:
    fields = obj(entry)
    model = cast(str, fields.get("model", ""))
    api_key = fields.get("api_key")
    key = api_key if isinstance(api_key, str) else None
    try:
        validated = VALIDATE(
            headers=headers(fields),
            model=model,
            messages=messages(fields),
            optional_params=at(fields, "optional_params"),
            litellm_params={},
            api_key=key,
            api_base=None,
        )
        url = COMPLETE_URL(
            api_base=None,
            api_key=key,
            model=model,
            optional_params=at(fields, "optional_params"),
            litellm_params={},
            stream=None,
        )
        body = TRANSFORM_REQUEST(
            model=model,
            messages=messages(fields),
            optional_params=at(fields, "optional_params"),
            litellm_params={},
            headers=validated,
        )
    except Exception:  # noqa: BLE001 - the status is what a caller can branch on
        return {"ok": False, "status": None}
    return {"ok": True, "value": {"url": url, "headers": validated, "body": body}}


def reply_entry(entry: Json) -> Json:
    fields = obj(entry)
    model = cast(str, fields.get("model", ""))
    created = cast(int, fields.get("created", 0))
    response = ModelResponse()
    raw_response = httpx.Response(
        cast(int, fields.get("status", 200)),
        headers=headers(fields),
        stream=httpx.ByteStream(cast(str, fields.get("body", "")).encode()),
    )
    raw_response.read()
    try:
        with patch(
            "litellm.llms.bytez.chat.transformation.time.time",
            return_value=float(created),
        ):
            result = TRANSFORM_RESPONSE(
                model=model,
                raw_response=raw_response,
                model_response=response,
                logging_obj=cast("Logging", None),
                request_data={},
                messages=messages(fields),
                optional_params={},
                litellm_params={},
                encoding=None,
            )
    except Exception as error:  # noqa: BLE001 - the status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    reply = cast(ModelResponse, result)
    choice = reply.choices[0]
    counted = cast(Usage, getattr(reply, "usage", None))
    hidden = cast("Mapping[str, object]", reply._hidden_params)
    additional = cast("Mapping[str, str]", hidden["additional_headers"])
    return {
        "ok": True,
        "value": {
            "created": reply.created,
            "model": reply.model,
            "content": choice.message.content,
            "usage": {
                "prompt_tokens": counted.prompt_tokens,
                "completion_tokens": counted.completion_tokens,
                "total_tokens": counted.total_tokens,
            },
            "provider_specific_fields": cast(
                Json, getattr(choice.message, "provider_specific_fields", None)
            ),
            "additional_headers": cast(Json, dict(additional)),
        },
    }


class Recording:
    def __init__(self, details: dict[str, object]) -> None:
        self.model_call_details = details


def chunk_projection(chunk: ModelResponseStream) -> Json:
    choice = chunk.choices[0]
    return {
        "content": choice.delta.content,
        "role": cast(Json, getattr(choice.delta, "role", None)),
        "finish_reason": choice.finish_reason,
        "index": choice.index,
        "model": chunk.model,
        "object": chunk.object,
        "id_prefix": chunk.id.startswith("chatcmpl-"),
    }


def chunks(fields: Object) -> Json:
    model = cast(str, fields.get("model", ""))
    provider = cast(str, fields.get("provider", ""))
    fragments = cast("list[str]", fields.get("fragments", []))
    details: dict[str, object] = {"litellm_params": {}, "custom_llm_provider": provider}
    outcomes: list[Json] = []
    with patch("litellm.suppress_debug_info", new=True):
        wrapper = BytezCustomStreamWrapper(
            completion_stream=iter(fragments),
            model=model,
            custom_llm_provider=provider,
            logging_obj=cast("Logging", Recording(details)),
        )
        for fragment in fragments:
            try:
                chunk = wrapper.chunk_creator(fragment)
            except StopIteration:
                outcomes.append({"end": True})
                continue
            except Exception:  # noqa: BLE001 - the status is what a caller can branch on
                outcomes.append({"failed": True})
                continue
            outcomes.append({"chunk": None if chunk is None else chunk_projection(chunk)})
    return outcomes


def entries(fields: Object, handler: Callable[[Json], Json]) -> Json:
    return [handler(entry) for entry in cast("list[Json]", fields.get("requests", []))]


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "options": options,
    "request": lambda f: entries(f, request_entry),
    "usage": usage,
    "reply": lambda f: entries(f, reply_entry),
    "chunks": chunks,
}


def run(argument: Json) -> Json:
    fields = obj(argument)
    handler = HANDLERS.get(cast(str, fields.get("function", "")))
    if handler is None:
        raise ValueError("unknown bytez_record function")
    return handler(fields)
