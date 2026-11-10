import json
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

import litellm
from litellm.llms.codestral.completion.handler import TextCompletionCodestralError
from litellm.types.utils import Logprobs, ModelResponseStream, TextCompletionResponse

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
PAIR_LENGTH = 2


class Choice(Protocol):
    index: int
    text: str | None
    finish_reason: str | None
    logprobs: object


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Reply(Protocol):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[Choice]
    usage: Usage


class Config(Protocol):
    get_config: Callable[[], Object]
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[..., Object]
    _chunk_parser: Callable[[str], Object]


class Handler(Protocol):
    process_text_completion_response: Callable[..., Reply]


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


CONFIG_FACTORY = cast("Callable[..., Config]", litellm.CodestralTextCompletionConfig)
HANDLER_FACTORY = cast("Callable[..., object]", litellm.CodestralTextCompletion)


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


def integer_field(fields: Object, name: str) -> int:
    return expect_integer(fields.get(name))


def pairs(value: Json) -> dict[str, Json]:
    result: dict[str, Json] = {}
    for entry in expect_list(value):
        pair = expect_list(entry)
        if len(pair) != PAIR_LENGTH:
            raise ValueError("a params entry needs a name and a value")
        result[expect_text(pair[0])] = pair[1]
    return result


def make_config(settings: Object) -> Config:
    return CONFIG_FACTORY(**settings)


def ignore_verbose(*_args: Json, **_kwargs: Json) -> None:
    pass


def logprobs_value(value: object) -> Json:
    if value is None:
        return None
    if isinstance(value, Logprobs):
        return cast(Json, value.model_dump())
    return cast(Json, value)


def project_reply(reply: Reply) -> Json:
    return {
        "id": reply.id,
        "object": reply.object,
        "created": reply.created,
        "model": reply.model,
        "usage": {
            "prompt_tokens": reply.usage.prompt_tokens,
            "completion_tokens": reply.usage.completion_tokens,
            "total_tokens": reply.usage.total_tokens,
        },
        "choices": [
            {
                "index": choice.index,
                "text": choice.text,
                "finish_reason": choice.finish_reason,
                "logprobs": logprobs_value(choice.logprobs),
            }
            for choice in reply.choices
        ],
    }


def error_value(error: NativeError) -> Json:
    return {
        "kind": "error",
        "status_code": error.status_code,
        "request": {"method": error.request.method, "url": str(error.request.url)},
        "response_status": error.response.status_code,
    }


def reply_step(step: Object) -> Json:
    body = text_field(step, "body")
    api_key_value = step.get("api_key")
    logger = FixtureLogger()
    handler = cast(Handler, HANDLER_FACTORY())
    try:
        result = handler.process_text_completion_response(
            model="codestral-latest",
            response=FixtureResponse(integer_field(step, "status"), body),
            model_response=TextCompletionResponse(),
            stream=False,
            logging_obj=logger,
            optional_params={},
            api_key="" if api_key_value is None else expect_text(api_key_value),
            data=step.get("request_data"),
            messages=expect_list(step.get("messages", [])),
            print_verbose=ignore_verbose,
            encoding=None,
        )
    except TextCompletionCodestralError as error:
        return {"events": logger.events, "outcome": error_value(cast(NativeError, error))}
    return {"events": logger.events, "outcome": {"kind": "reply", "reply": project_reply(result)}}


def project_stream(dumped: Object) -> Json:
    projected: list[Json] = []
    for raw_choice in expect_list(dumped.get("choices", [])):
        choice = expect_object(raw_choice)
        delta = expect_object(choice.get("delta", {}))
        projected.append({
            "index": choice.get("index"),
            "finish_reason": choice.get("finish_reason"),
            "logprobs": logprobs_value(choice.get("logprobs")),
            "delta": {
                "content": delta.get("content"),
                "role": delta.get("role"),
                "function_call": delta.get("function_call"),
                "tool_calls": delta.get("tool_calls"),
                "audio": delta.get("audio"),
            },
        })
    value: Object = {
        "id": dumped.get("id"),
        "created": dumped.get("created"),
        "object": dumped.get("object"),
        "model": dumped.get("model"),
        "system_fingerprint": dumped.get("system_fingerprint"),
        "provider_specific_fields": dumped.get("provider_specific_fields"),
        "choices": projected,
    }
    usage = dumped.get("usage")
    if usage is not None:
        counters = expect_object(usage)
        value["usage"] = {
            "prompt_tokens": counters.get("prompt_tokens"),
            "completion_tokens": counters.get("completion_tokens"),
            "total_tokens": counters.get("total_tokens"),
        }
    return value


def project_chunk(result: Object) -> Json:
    value: Object = {
        "text": result.get("text"),
        "is_finished": result.get("is_finished"),
        "finish_reason": result.get("finish_reason"),
    }
    original = result.get("original_chunk")
    if original is None:
        return value
    dumped = cast(
        Object,
        cast(ModelResponseStream, original).model_dump(),  # pyright: ignore[reportUnknownMemberType]
    )
    value["logprobs"] = result.get("logprobs")
    value["original_chunk"] = project_stream(dumped)
    return value


def chunk_step(step: Object) -> Json:
    config = make_config({})
    return project_chunk(config._chunk_parser(text_field(step, "line")))


def execute_step(step: Object, config: Config) -> Json:
    kind = text_field(step, "kind")
    if kind == "config":
        return config.get_config()
    if kind == "supported_params":
        model = text_field(step, "model")
        return {
            "model": model,
            "supported": cast(Json, config.get_supported_openai_params(model)),
        }
    if kind == "map_params":
        optional = expect_object(step.get("optional_params", {}))
        return config.map_openai_params(
            pairs(step.get("params", [])),
            optional,
            text_field(step, "model"),
            drop_params=False,
        )
    if kind == "reply":
        return reply_step(step)
    if kind == "chunk":
        return chunk_step(step)
    raise ValueError(f"unknown codestral_records step: {kind}")


def run(argument: Json) -> Json:
    fields = expect_object(argument)
    settings_value = fields.get("settings")
    settings = {} if settings_value is None else expect_object(settings_value)
    config = make_config(settings)
    return [
        execute_step(expect_object(step), config)
        for step in expect_list(fields.get("steps"))
    ]
