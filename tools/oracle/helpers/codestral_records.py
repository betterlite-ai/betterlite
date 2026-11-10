import json
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

import litellm
from litellm.llms.codestral.completion.handler import TextCompletionCodestralError
from litellm.types.utils import Logprobs, TextCompletionResponse

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
PAIR_LENGTH = 2


class Config(Protocol):
    get_config: Callable[[], Object]
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[..., Object]
    _chunk_parser: Callable[[str], Object]


class Dumpable(Protocol):
    def model_dump(self) -> Json: ...


class Handler(Protocol):
    process_text_completion_response: Callable[..., Dumpable]


class Request(Protocol):
    method: str
    url: object


class ResponseCode(Protocol):
    status_code: int


class NativeError(Protocol):
    status_code: int
    request: Request
    response: ResponseCode


class FixtureResponse:
    def __init__(self, status_code: int, text: str) -> None:
        self.status_code = status_code
        self.text = text

    def json(self) -> Object:
        return cast(Object, json.loads(self.text))


class FixtureLogger:
    def __init__(self) -> None:
        self.events: list[Json] = []

    def post_call(self, **params: Json) -> None:
        additional = params.get("additional_args")
        complete_input = None
        if isinstance(additional, dict):
            complete_input = additional.get("complete_input_dict")
        self.events.append({
            "name": "post_call",
            "input": params.get("input"),
            "api_key": params.get("api_key"),
            "original_response": params.get("original_response"),
            "complete_input_dict": complete_input,
        })


CONFIG = cast("Callable[..., Config]", litellm.CodestralTextCompletionConfig)
HANDLER = cast("Callable[..., Handler]", litellm.CodestralTextCompletion)


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_integer(value: Json) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError("expected an integer")
    return value


def text_field(fields: Object, name: str) -> str:
    return expect_text(fields.get(name))


def pairs(value: Json) -> dict[str, Json]:
    result: dict[str, Json] = {}
    for entry in expect_list(value):
        pair = expect_list(entry)
        if len(pair) != PAIR_LENGTH:
            raise ValueError("a params entry needs a name and a value")
        result[expect_text(pair[0])] = pair[1]
    return result


def ignore_verbose(*_args: Json, **_kwargs: Json) -> None:
    pass


def logprobs_value(value: object) -> Json:
    if isinstance(value, Logprobs):
        return cast(Json, value.model_dump())
    return cast(Json, value)


def project_reply(result: Dumpable) -> Json:
    dumped = expect_object(result.model_dump())
    choices: list[Json] = [
        {
            "index": choice.get("index"),
            "text": choice.get("text"),
            "finish_reason": choice.get("finish_reason"),
            "logprobs": logprobs_value(choice.get("logprobs")),
        }
        for choice in map(expect_object, expect_list(dumped.get("choices")))
    ]
    usage = expect_object(dumped.get("usage"))
    return {
        "id": dumped.get("id"),
        "object": dumped.get("object"),
        "created": dumped.get("created"),
        "model": dumped.get("model"),
        "usage": {
            "prompt_tokens": usage.get("prompt_tokens"),
            "completion_tokens": usage.get("completion_tokens"),
            "total_tokens": usage.get("total_tokens"),
        },
        "choices": choices,
    }


def reply_step(step: Object) -> Json:
    body = text_field(step, "body")
    api_key_value = step.get("api_key")
    messages_value = step.get("messages")
    logger = FixtureLogger()
    try:
        result = HANDLER().process_text_completion_response(
            model="codestral-latest",
            response=FixtureResponse(expect_integer(step.get("status")), body),
            model_response=TextCompletionResponse(),
            stream=False,
            logging_obj=logger,
            optional_params={},
            api_key="" if api_key_value is None else expect_text(api_key_value),
            data=step.get("request_data"),
            messages=[] if messages_value is None else expect_list(messages_value),
            print_verbose=ignore_verbose,
            encoding=None,
        )
    except TextCompletionCodestralError as error:
        native = cast(NativeError, error)
        outcome: Json = {
            "kind": "error",
            "status_code": native.status_code,
            "request": {"method": native.request.method, "url": str(native.request.url)},
            "response_status": native.response.status_code,
        }
        return {"events": logger.events, "outcome": outcome}
    return {"events": logger.events, "outcome": {"kind": "reply", "reply": project_reply(result)}}


def chunk_step(step: Object) -> Json:
    parsed = CONFIG()._chunk_parser(text_field(step, "line"))
    return {
        "text": parsed.get("text"),
        "is_finished": parsed.get("is_finished"),
        "finish_reason": parsed.get("finish_reason"),
        "logprobs": parsed.get("logprobs"),
    }


def options_value(fields: Object, config: Config) -> Json:
    optional = expect_object(fields.get("optional_params", {}))
    model = text_field(fields, "model")
    return {
        "model": model,
        "config": config.get_config(),
        "supported": cast(Json, config.get_supported_openai_params(model)),
        "mapped": config.map_openai_params(
            pairs(fields.get("params", [])),
            optional,
            model,
            drop_params=False,
        ),
    }


def execute_step(step: Object, config: Config) -> Json:
    kind = text_field(step, "kind")
    if kind == "options":
        return options_value(step, config)
    if kind == "reply":
        return reply_step(step)
    if kind == "chunk":
        return chunk_step(step)
    raise ValueError(f"unknown codestral_records step: {kind}")


def run(argument: Json) -> Json:
    fields = expect_object(argument)
    settings_value = fields.get("settings")
    settings = {} if settings_value is None else expect_object(settings_value)
    config = CONFIG(**settings)
    return [
        execute_step(expect_object(step), config)
        for step in expect_list(fields.get("steps"))
    ]
