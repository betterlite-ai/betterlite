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
SUPPORTED = cast("Callable[[str], list[str]]", CONFIG.get_supported_openai_params)
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


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def integer(fields: Object, name: str) -> int:
    value = fields.get(name)
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    return value


def listing(fields: Object, name: str) -> list[Json]:
    value = fields.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return value


def strings(fields: Object, name: str) -> list[str]:
    items = listing(fields, name)
    if not all(isinstance(item, str) for item in items):
        raise TypeError(f"{name} must hold strings")
    return [cast(str, item) for item in items]


def header_map(fields: Object) -> dict[str, str]:
    return {
        name: value
        for name, value in obj(fields.get("headers", {})).items()
        if isinstance(value, str)
    }


def messages(fields: Object) -> "list[AllMessageValues]":
    return cast("list[AllMessageValues]", listing(fields, "messages"))


def options(fields: Object) -> Json:
    model = text(fields, "supported_model")
    mapped: list[Json] = []
    for entry in listing(fields, "mappings"):
        mapping = obj(entry)
        try:
            adapted = MAP_PARAMS(
                obj(mapping.get("non_default_params", {})),
                obj(mapping.get("optional_params", {})),
                model,
                bool(mapping.get("drop_params", False)),
            )
        except Exception:  # noqa: BLE001 - the status is what a caller can branch on
            mapped.append({"ok": False})
            continue
        mapped.append({"ok": True, "value": adapted})
    return {"supported": cast(Json, list(SUPPORTED(model))), "mapped": mapped}


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
    model = text(fields, "model")
    api_key = fields.get("api_key")
    key = api_key if isinstance(api_key, str) else None
    try:
        validated = VALIDATE(
            headers=header_map(fields),
            model=model,
            messages=messages(fields),
            optional_params=obj(fields.get("optional_params", {})),
            litellm_params={},
            api_key=key,
            api_base=None,
        )
        url = COMPLETE_URL(
            api_base=None,
            api_key=key,
            model=model,
            optional_params=obj(fields.get("optional_params", {})),
            litellm_params={},
            stream=None,
        )
        body = TRANSFORM_REQUEST(
            model=model,
            messages=messages(fields),
            optional_params=obj(fields.get("optional_params", {})),
            litellm_params={},
            headers=validated,
        )
    except Exception:  # noqa: BLE001 - the status is what a caller can branch on
        return {"ok": False, "status": None}
    return {"ok": True, "value": {"url": url, "headers": validated, "body": body}}


def reply_entry(entry: Json) -> Json:
    fields = obj(entry)
    body = text(fields, "body")
    status = integer(fields, "status")
    model = text(fields, "model")
    created = integer(fields, "created")
    response = ModelResponse()
    try:
        with patch(
            "litellm.llms.bytez.chat.transformation.time.time",
            return_value=float(created),
        ):
            result = cast(
                ModelResponse,
                TRANSFORM_RESPONSE(
                    model=model,
                    raw_response=httpx.Response(status, text=body, headers=header_map(fields)),
                    model_response=response,
                    logging_obj=cast("Logging", None),
                    request_data={},
                    messages=messages(fields),
                    optional_params={},
                    litellm_params={},
                    encoding=None,
                ),
            )
    except Exception as error:  # noqa: BLE001 - the status is what a caller can branch on
        return {"ok": False, "status": cast(Json, getattr(error, "status_code", None))}
    choice = result.choices[0]
    counted = cast(Usage, getattr(result, "usage", None))
    hidden = cast("Mapping[str, object]", result._hidden_params)
    additional = cast("Mapping[str, str]", hidden["additional_headers"])
    return {
        "ok": True,
        "value": {
            "created": result.created,
            "model": result.model,
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
    model = text(fields, "model")
    provider = text(fields, "provider")
    fragments = strings(fields, "fragments")
    details: dict[str, object] = {"litellm_params": {}, "custom_llm_provider": provider}
    logging_obj = cast("Logging", Recording(details))
    wrapper = BytezCustomStreamWrapper(
        completion_stream=iter(fragments),
        model=model,
        custom_llm_provider=provider,
        logging_obj=logging_obj,
    )
    outcomes: list[Json] = []
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


HANDLERS: dict[str, Callable[[Object], Json]] = {
    "options": options,
    "request": lambda f: [request_entry(entry) for entry in listing(f, "requests")],
    "usage": usage,
    "reply": lambda f: [reply_entry(entry) for entry in listing(f, "requests")],
    "chunks": chunks,
}


def run(argument: Json) -> Json:
    fields = obj(argument)
    handler = HANDLERS.get(text(fields, "function"))
    if handler is None:
        raise ValueError("unknown bytez_record function")
    return handler(fields)
