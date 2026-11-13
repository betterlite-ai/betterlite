from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.predibase.chat import transformation
from litellm.llms.predibase.chat.transformation import PredibaseConfig
from litellm.llms.predibase.common_utils import PredibaseError
from litellm.types.utils import ModelResponse

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Object: TypeAlias = dict[str, Json]
FIXED_NOW = 1_700_000_000.0
DEFAULT_MAX_NEW_TOKENS = 4096

CONFIG = PredibaseConfig()
SUPPORTED = cast("Callable[..., list[str]]", CONFIG.get_supported_openai_params)
GET_CONFIG = cast("Callable[[], Object]", CONFIG.get_config)
MAP_PARAMS = cast("Callable[..., Object]", CONFIG.map_openai_params)
TRANSFORM_REQUEST = cast("Callable[..., Object]", CONFIG.transform_request)
COMPLETE_URL = cast("Callable[..., str]", CONFIG.get_complete_url)
VALIDATE = cast("Callable[..., Object]", CONFIG.validate_environment)
GET_ERROR_CLASS = cast("Callable[..., PredibaseError]", CONFIG.get_error_class)
TRANSFORM_RESPONSE = cast("Callable[..., object]", CONFIG.transform_response)
OUTPUT_PARSER = cast("Callable[[str], str]", CONFIG.output_parser)


class ChoiceMessage(Protocol):
    content: str | None


class Choice(Protocol):
    message: ChoiceMessage
    finish_reason: str
    index: int


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Completion(Protocol):
    choices: list[Choice]
    created: int
    model: str
    usage: Usage
    _hidden_params: dict[str, Json]


class Logging:
    def post_call(self, **params: Json) -> None:
        del params


class Encoding:
    def __init__(self, *, fail: bool) -> None:
        self.fail = fail

    def encode(self, text: str) -> list[str]:
        if self.fail:
            raise RuntimeError("encode failed")
        return list(text)


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_text_or_none(value: Json) -> str | None:
    return None if value is None else expect_text(value)


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def expect_bool(value: Json) -> bool:
    if not isinstance(value, bool):
        raise TypeError("expected a boolean")
    return value


def expect_int(value: Json) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("expected an integer")
    return value


def optional_int(value: Json, default: int) -> int:
    if value is None:
        return default
    return expect_int(value)


def optional_bool(value: Json, *, default: bool) -> bool:
    if value is None:
        return default
    return expect_bool(value)


def status_of(error: Exception) -> int:
    if isinstance(error, PredibaseError):
        return int(error.status_code)
    return 500


@contextmanager
def with_max_new_tokens(value: int) -> Generator[None]:
    saved = PredibaseConfig.max_new_tokens
    PredibaseConfig.max_new_tokens = value
    try:
        yield
    finally:
        PredibaseConfig.max_new_tokens = saved


def count_messages_standin(**params: Json) -> int:
    messages = params.get("messages")
    if not isinstance(messages, list):
        return 0
    return len(messages)


def failing_counter(**params: Json) -> int:
    del params
    raise RuntimeError("the token counter failed")


def map_params(input_value: Object) -> Json:
    max_new_tokens = optional_int(input_value.get("max_new_tokens"), DEFAULT_MAX_NEW_TOKENS)
    cases = [expect_object(entry) for entry in expect_list(input_value["cases"])]
    results: list[Json] = []
    with with_max_new_tokens(max_new_tokens):
        config: Json = GET_CONFIG()
        for entry in cases:
            non_default = dict(expect_object(entry.get("non_default") or {}))
            options = MAP_PARAMS(
                non_default_params=non_default,
                optional_params={},
                model="model",
                drop_params=False,
            )
            results.append({"options": cast("Json", options)})
    return {"results": results, "supported": list(SUPPORTED(model="model")), "config": config}


def payload(input_value: Object) -> Json:
    max_new_tokens = optional_int(input_value.get("max_new_tokens"), DEFAULT_MAX_NEW_TOKENS)
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    custom_prompt_dict = expect_object(input_value.get("custom_prompt_dict") or {})
    bodies: list[Json] = []
    with with_max_new_tokens(max_new_tokens):
        for entry in requests:
            try:
                body = TRANSFORM_REQUEST(
                    model=expect_text(entry["model"]),
                    messages=cast("list[dict[str, Json]]", expect_list(entry["messages"])),
                    optional_params=dict(expect_object(entry.get("options") or {})),
                    litellm_params={"custom_prompt_dict": custom_prompt_dict},
                    headers={},
                )
                bodies.append({"ok": True, "body": cast("Json", body)})
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                bodies.append({"ok": False, "status": status_of(error)})
    return {"bodies": bodies}


def url(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    urls: list[Json] = []
    for entry in requests:
        tenant = expect_text_or_none(entry.get("tenant"))
        litellm_params: Object = {} if tenant is None else {"predibase_tenant_id": tenant}
        try:
            found = COMPLETE_URL(
                api_base=expect_text_or_none(entry.get("api_base")), api_key=None,
                model=expect_text(entry["model"]),
                optional_params=dict(expect_object(entry.get("options") or {})),
                litellm_params=litellm_params,
                stream=None if entry.get("stream") is None else expect_bool(entry["stream"]),
            )
            urls.append({"ok": True, "url": found})
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            urls.append({"ok": False, "status": status_of(error)})
    return {"urls": urls}


def headers(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    found: list[Json] = []
    for entry in requests:
        try:
            merged = VALIDATE(
                dict(expect_object(entry.get("headers") or {})),
                "model", [],
                {}, {},
                api_key=expect_text_or_none(entry.get("api_key")),
            )
            found.append({"ok": True, "headers": cast("Json", merged)})
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            found.append({"ok": False, "status": status_of(error)})
    class_error = GET_ERROR_CLASS("boom", 418, {"x-a": "1"})
    class_headers: Json = {
        "status_code": int(class_error.status_code),
        "headers": cast("Json", class_error.headers),
    }
    return {"requests": found, "error_class": class_headers}


def describe(result: Completion) -> Json:
    choices: list[Json] = [
        {
            "index": choice.index, "content": choice.message.content,
            "finish_reason": choice.finish_reason,
            "logprobs": cast("Json", getattr(choice.message, "logprobs", None)),
            "logprob": cast("Json", getattr(choice.message, "_logprob", None)),
        }
        for choice in result.choices
    ]
    hidden = result._hidden_params  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    return {
        "ok": True, "model": result.model,
        "created": result.created,
        "usage": {
            "prompt_tokens": result.usage.prompt_tokens,
            "completion_tokens": result.usage.completion_tokens,
            "total_tokens": result.usage.total_tokens,
        },
        "additional_headers": hidden.get("additional_headers", {}),
        "choices": choices,
    }


def one_reply(entry: Object) -> Json:
    body = expect_text(entry["body"])
    response = httpx.Response(
        optional_int(entry.get("status"), 200), content=body.encode(),
        headers={
            key: expect_text(value)
            for key, value in expect_object(entry.get("headers") or {}).items()
        },
        request=httpx.Request("POST", "https://host"),
    )
    encoding = Encoding(fail=optional_bool(entry.get("encoding_fails"), default=False))
    counter = (
        failing_counter
        if optional_bool(entry.get("counter_fails"), default=False)
        else count_messages_standin
    )
    clock = FIXED_NOW if entry.get("now") is None else float(expect_int(entry["now"]))
    with (
        patch.object(transformation.time, "time", return_value=clock),  # pyright: ignore[reportPrivateImportUsage]
        patch.object(litellm, "token_counter", counter),
    ):
        try:
            result = cast(
                "Completion",
                TRANSFORM_RESPONSE(
                    model=expect_text(entry["model"]),
                    raw_response=response,
                    model_response=ModelResponse(),
                    logging_obj=Logging(),
                    request_data={
                        "inputs": None,
                        "parameters": dict(expect_object(entry.get("parameters") or {})),
                    },
                    messages=cast(
                        "list[dict[str, Json]]", expect_list(entry.get("messages") or [])
                    ),
                    optional_params=dict(expect_object(entry.get("options") or {})),
                    litellm_params={},
                    encoding=encoding,
                ),
            )
        except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
            return {"ok": False, "status": status_of(error)}
    return describe(result)


def reply(input_value: Object) -> Json:
    replies = [expect_object(entry) for entry in expect_list(input_value["replies"])]
    return {"replies": [one_reply(entry) for entry in replies]}


def output_parser(input_value: Object) -> Json:
    texts = [expect_text(text) for text in expect_list(input_value["texts"])]
    return {"texts": [OUTPUT_PARSER(text) for text in texts]}


OPS: dict[str, Callable[[Object], Json]] = {
    "map_params": map_params, "payload": payload, "url": url,
    "headers": headers, "reply": reply, "output_parser": output_parser,
}


def run(input_value: Object) -> Json:
    operation = expect_text(input_value["op"])
    return OPS[operation](input_value)
