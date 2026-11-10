from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm import COHERE_DEFAULT_EMBEDDING_INPUT_TYPE
from litellm.llms.cohere.embed.transformation import CohereEmbeddingConfig
from litellm.types.utils import EmbeddingResponse, Usage

if TYPE_CHECKING:
    from collections.abc import Callable

    from litellm.litellm_core_utils.litellm_logging import Logging

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

CONFIG = CohereEmbeddingConfig()
SUPPORTED = cast("Callable[..., list[str]]", CONFIG.get_supported_openai_params)
MAP = cast("Callable[..., Object]", CONFIG.map_openai_params)
VALIDATE = cast("Callable[..., Object]", CONFIG.validate_environment)
URL = cast("Callable[..., str]", CONFIG.get_complete_url)
TRANSFORM = cast("Callable[..., Object]", CONFIG._transform_request)
REQUEST = cast("Callable[..., Object]", CONFIG.transform_embedding_request)
RESPONSE = cast("Callable[..., EmbeddingResponse]", CONFIG.transform_embedding_response)


def text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def optional(value: Json) -> str | None:
    return None if value is None else text(value)


def names(value: Json) -> list[str]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return [text(item) for item in value]


class Encoding:
    def __init__(self, fail_on: str | None) -> None:
        self.fail_on = fail_on
        self.encoded: list[str] = []

    def encode(self, value: str, /) -> list[int]:
        self.encoded.append(value)
        if value == self.fail_on:
            raise RuntimeError("the fixture tokenizer refused this text")
        return list(range(len(value)))


class Recorder:
    def __init__(self, input_value: list[str]) -> None:
        self.model_call_details: dict[str, Json] = {"input": cast(Json, input_value)}
        self.calls: list[Json] = []

    def post_call(self, **params: Json) -> None:
        additional = params.get("additional_args")
        complete = additional.get("complete_input_dict") if isinstance(additional, dict) else None
        self.calls.append(
            {
                "input": params.get("input"),
                "api_key": params.get("api_key"),
                "complete_input_dict": complete,
                "original_response": params.get("original_response"),
            }
        )


def result(value: Object | None) -> Json:
    return {"ok": True, "value": value} if value is not None else {"ok": False, "value": None}


def wrapper(model: str, raw: Json, mapped: Object) -> Json:
    try:
        return result(REQUEST(model, raw, mapped, {}))
    except Exception:  # noqa: BLE001 - both sides report a refusal as a failed outcome
        return result(None)


def lower(model: str, raw: Json, mapped: Object) -> Json:
    if not isinstance(raw, list) or not all(isinstance(item, str) for item in raw):
        return None
    try:
        return result(TRANSFORM(model, cast(list[str], raw), mapped))
    except Exception:  # noqa: BLE001 - both sides report a refusal as a failed outcome
        return result(None)


def request(call: Object) -> Json:
    model = text(call["model"])
    mapped = MAP(dict(obj(call.get("options", {}))), {}, model, drop_params=False)
    mapped.update(obj(call.get("inference", {})))
    key = optional(call.get("api_key"))
    calls = [
        {"wrapper": wrapper(model, raw, mapped), "lower": lower(model, raw, mapped)}
        for raw in cast(list[Json], call.get("inputs", []))
    ]
    return cast(
        Json,
        {
            "default_input_type": COHERE_DEFAULT_EMBEDDING_INPUT_TYPE,
            "supported": list(SUPPORTED(model)),
            "options": mapped,
            "headers": VALIDATE(obj(call.get("headers", {})), model, [], {}, {}, api_key=key),
            "url": URL(optional(call.get("api_base")), key, model, {}, {}),
            "calls": calls,
        },
    )


def projection(response: EmbeddingResponse) -> Json:
    usage = cast(Usage, response.usage)
    details = usage.prompt_tokens_details
    billed: Json = (
        None
        if details is None
        else cast(Json, {"text_tokens": details.text_tokens, "image_tokens": details.image_tokens})
    )
    return cast(
        Json,
        {
            "object": response.object,
            "data": cast(Json, response.data),
            "model": response.model,
            "usage": {
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
                "prompt_tokens_details": billed,
            },
        },
    )


def reply(model: str, entry: Object) -> Json:
    input_value = names(entry.get("input", []))
    mapped = MAP(dict(obj(entry.get("options", {}))), {}, model, drop_params=False)
    data = REQUEST(model, input_value, mapped, {})
    encoding = Encoding(optional(entry.get("fail_on")))
    recorder = Recorder(input_value)
    try:
        with patch.object(litellm, "encoding", encoding):
            response = RESPONSE(
                model=model,
                raw_response=httpx.Response(200, content=text(entry["body"])),
                model_response=EmbeddingResponse(),
                logging_obj=cast("Logging", recorder),
                api_key=optional(entry.get("api_key")),
                request_data=data,
                optional_params={},
                litellm_params={},
            )
    except Exception:  # noqa: BLE001 - both sides report a refusal as a failed outcome
        return cast(
            Json,
            {"ok": False, "value": None, "post_call": recorder.calls, "encoded": encoding.encoded},
        )
    return cast(
        Json,
        {
            "ok": True,
            "value": projection(response),
            "post_call": recorder.calls,
            "encoded": encoding.encoded,
        },
    )


def run(input_value: Object) -> Json:
    function = input_value.get("function")
    if function == "request":
        return request(input_value)
    if function == "response":
        calls = [
            reply(text(input_value["model"]), obj(entry))
            for entry in cast(list[Json], input_value.get("requests", []))
        ]
        return cast(Json, {"calls": calls})
    raise ValueError(f"unsupported function {function}")
