from contextlib import suppress
from typing import TYPE_CHECKING, NotRequired, TypeAlias, TypedDict, cast
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


class Reply(TypedDict):
    body: str
    input: list[str]
    options: NotRequired[Object]
    api_key: NotRequired[str | None]
    fail_on: NotRequired[str | None]


class Call(TypedDict):
    function: str
    model: str
    options: NotRequired[Object]
    inference: NotRequired[Object]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    headers: NotRequired[Object]
    inputs: NotRequired[list[Json]]
    requests: NotRequired[list[Reply]]


CONFIG = CohereEmbeddingConfig()
MAP = cast("Callable[..., Object]", CONFIG.map_openai_params)
REQUEST = cast("Callable[..., Object]", CONFIG.transform_embedding_request)
LOWER = cast("Callable[..., Object]", CONFIG._transform_request)
HEADERS = cast("Callable[..., Object]", CONFIG.validate_environment)
URL = cast("Callable[..., str]", CONFIG.get_complete_url)
RESPONSE = cast("Callable[..., EmbeddingResponse]", CONFIG.transform_embedding_response)


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
        self.model_call_details: dict[str, Json] = {"input": cast("Json", input_value)}
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


def attempt(function: "Callable[..., Json]", *args: Json) -> Json:
    try:
        return {"ok": True, "value": function(*args)}
    except Exception:  # noqa: BLE001 - both sides report a refusal as a failed outcome
        return {"ok": False, "value": None}


def request(call: Call) -> Json:
    model = call["model"]
    mapped = MAP(dict(call.get("options", {})), {}, model, drop_params=False)
    mapped.update(call.get("inference", {}))
    key = call.get("api_key")
    calls: list[Json] = []
    for raw in call.get("inputs", []):
        texts = isinstance(raw, list) and all(isinstance(item, str) for item in raw)
        lower = attempt(LOWER, model, raw, mapped) if texts else None
        calls.append({"wrapper": attempt(REQUEST, model, raw, mapped, {}), "lower": lower})
    headers = HEADERS(call.get("headers", {}), model, [], {}, {}, api_key=key)
    return cast(
        "Json",
        {
            "default_input_type": COHERE_DEFAULT_EMBEDDING_INPUT_TYPE,
            "supported": CONFIG.get_supported_openai_params(model),
            "options": mapped,
            "headers": headers,
            "url": URL(call.get("api_base"), key, model, {}, {}),
            "calls": calls,
        },
    )


def reply(model: str, entry: Reply) -> Json:
    mapped = MAP(dict(entry.get("options", {})), {}, model, drop_params=False)
    data = REQUEST(model, entry["input"], mapped, {})
    encoding = Encoding(entry.get("fail_on"))
    recorder = Recorder(entry["input"])
    outcome: Object = {"ok": False, "value": None}
    with suppress(Exception), patch.object(litellm, "encoding", encoding):
        response = RESPONSE(
            model,
            httpx.Response(200, content=entry["body"]),
            EmbeddingResponse(),
            cast("Logging", recorder),
            entry.get("api_key"),
            data,
            {},
            {},
        )
        usage = cast("Usage", response.usage)
        details = usage.prompt_tokens_details
        billed: Json = None if details is None else [details.text_tokens, details.image_tokens]
        value: Object = {
            "object": response.object,
            "data": cast("Json", response.data),
            "model": response.model,
            "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
            "billed": billed,
        }
        outcome = {"ok": True, "value": cast("Json", value)}
    return {**outcome, "post_call": recorder.calls, "encoded": cast("Json", encoding.encoded)}


def run(input_value: Object) -> Json:
    call = cast("Call", input_value)
    if call["function"] == "request":
        return request(call)
    return {"calls": [reply(call["model"], entry) for entry in call.get("requests", [])]}
