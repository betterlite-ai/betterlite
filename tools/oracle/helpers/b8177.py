import json
import uuid
from collections.abc import Callable, Iterator, Mapping
from typing import TypeAlias, cast
from unittest.mock import MagicMock, patch

import httpx
from litellm.llms.gigachat.chat import streaming, transformation
from litellm.llms.gigachat.chat.streaming import GigaChatModelResponseIterator
from litellm.llms.gigachat.chat.transformation import GigaChatConfig
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

CONFIG = GigaChatConfig()
MAP_PARAMS = cast("Callable[..., Object]", CONFIG.map_openai_params)
VALIDATE_ENVIRONMENT = cast("Callable[..., Object]", CONFIG.validate_environment)
COMPLETE_URL = cast("Callable[..., str]", CONFIG.get_complete_url)
TRANSFORM_REQUEST = cast("Callable[..., Object]", CONFIG.transform_request)
TRANSFORM_RESPONSE = cast("Callable[..., ModelResponse]", CONFIG.transform_response)


class FixedUuidModule:
    def uuid4(self) -> uuid.UUID:
        return uuid.UUID("01234567-89ab-4def-8123-456789abcdef")


class Uploads:
    def __init__(self, ids: dict[str, str]) -> None:
        self.ids = ids

    def __call__(self, *, image_url: str, **_options: object) -> str | None:
        return self.ids.get(image_url)


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def items(value: Json) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return [obj(item) for item in value]


def pick(value: Json, *names: str) -> Object:
    fields = obj(value)
    return {name: fields.get(name) for name in names}


def failure(error: Exception) -> Json:
    carried = cast("Json", getattr(error, "status_code", None))
    status = carried if isinstance(carried, int) and not isinstance(carried, bool) else None
    return {"ok": False, "status_code": status}


def settled(value: Json) -> Json:
    if isinstance(value, list):
        return [settled(item) for item in value]
    if isinstance(value, dict):
        return {key: settled(item) for key, item in value.items()}
    if isinstance(value, str) and value.startswith(("{", "[")):
        try:
            return settled(cast("Json", json.loads(value)))
        except json.JSONDecodeError:
            return value
    return value


def usage_value(block: Json) -> Json:
    if block is None:
        return None
    fields = obj(block)
    details = fields.get("prompt_tokens_details")
    return {
        **pick(fields, "prompt_tokens", "completion_tokens", "total_tokens"),
        "cached_tokens": None if details is None else obj(details).get("cached_tokens"),
    }


def tool_value(call: Object) -> Object:
    function = obj(call.get("function"))
    return {
        **pick(call, "id", "type", "index"),
        "function": {
            "name": function.get("name"),
            "arguments": function.get("arguments"),
        },
    }


def reply_value(result: ModelResponse) -> Json:
    dumped = obj(cast("Json", json.loads(result.model_dump_json())))
    choices: list[Json] = []
    for choice in items(dumped.get("choices")):
        message = obj(choice.get("message"))
        calls = message.get("tool_calls")
        choices.append(
            {
                **pick(choice, "index", "finish_reason", "provider_specific_fields"),
                **pick(message, "role", "content"),
                "tool_calls": None
                if calls is None
                else [
                    {key: value for key, value in tool_value(call).items() if key != "index"}
                    for call in items(calls)
                ],
            }
        )
    return {
        "ok": True,
        **pick(dumped, "id", "created", "model"),
        "choices": choices,
        "usage": usage_value(dumped.get("usage")),
    }


def chunk_value(chunk: Mapping[str, Json]) -> Json:
    tool = chunk.get("tool_use")
    return {
        **pick(dict(chunk), "text", "is_finished", "finish_reason", "index"),
        "tool_use": None if tool is None else tool_value(obj(tool)),
        "usage": usage_value(chunk.get("usage")),
    }


def map_params(step: Object) -> Json:
    options = obj(step.get("optional_params") or {})
    try:
        mapped = MAP_PARAMS(obj(step.get("params")), dict(options), "", drop_params=False)
    except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
        return failure(error)
    return {"ok": True, "params": mapped}


def request(step: Object) -> Json:
    model = cast("str", step["model"])
    messages = cast("list[Json]", step["messages"])
    params = obj(step.get("params") or {})
    with (
        patch.object(transformation, "get_access_token", return_value=step["token"]),
        patch.object(
            transformation,
            "upload_file_sync",
            Uploads(cast("dict[str, str]", step.get("uploads") or {})),
        ),
    ):
        try:
            headers = VALIDATE_ENVIRONMENT(
                cast("dict[str, Json]", step.get("headers") or {}),
                model,
                messages,
                params,
                {},
                api_key=None,
            )
            url = COMPLETE_URL(
                api_base=cast("str | None", step.get("api_base")),
                api_key=None,
                model=model,
                optional_params=params,
                litellm_params={},
                stream=None,
            )
            body = TRANSFORM_REQUEST(model, messages, params, {}, headers)
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return {"ok": True, "url": url, "headers": headers, "body": body}


def reply(step: Object) -> Json:
    response = httpx.Response(
        cast("int", step["status"]),
        content=cast("str", step["body"]).encode(),
        request=httpx.Request("POST", "https://host"),
    )
    options: Object = {"_structured_output": True} if step.get("structured_output") else {}
    with (
        patch.object(transformation.time, "time", return_value=step["created"]),  # pyright: ignore[reportPrivateImportUsage]
        patch.object(transformation, "uuid", FixedUuidModule()),
    ):
        try:
            result = TRANSFORM_RESPONSE(
                model=cast("str", step["model"]),
                raw_response=response,
                model_response=ModelResponse(),
                logging_obj=MagicMock(),
                request_data={},
                messages=[],
                optional_params=options,
                litellm_params={},
                encoding=None,
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return reply_value(result)


def stream(step: Object) -> Json:
    events = cast("list[str]", step["events"])
    iterator = cast(
        "Iterator[Mapping[str, Json]]",
        GigaChatModelResponseIterator(iter(events), sync_stream=True),
    )
    with patch.object(streaming, "uuid", FixedUuidModule()):
        try:
            chunks = [chunk_value(chunk) for chunk in iterator]
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return failure(error)
    return {"ok": True, "chunks": chunks}


STEPS: dict[str, Callable[[Object], Json]] = {
    "map_params": map_params,
    "request": request,
    "reply": reply,
    "stream": stream,
}


def run(value: dict[str, Json]) -> Json:
    return settled([STEPS[cast("str", step["kind"])](step) for step in items(value["steps"])])
