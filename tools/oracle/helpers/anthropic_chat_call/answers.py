from typing import Final

import httpx
from litellm.llms.anthropic.chat.handler import ModelResponseIterator
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

from .wire import Attempt, JsonObject, Recorder, result_of

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class MessageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    content: str | None


class ChoiceView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    message: MessageView
    finish_reason: str | None


class ReplyView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    model: str | None
    choices: list[ChoiceView]


class OptionsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    json_mode: bool | None
    speed: str | None
    tool_name_reverse_map: dict[str, str] | None


class StreamView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    model: str | None
    custom_llm_provider: str | None
    completion_stream: ModelResponseIterator


def request_body(content: bytes) -> JsonValue:
    try:
        return JSON.validate_json(content)
    except ValidationError:
        return None


def selected(headers: httpx.Headers, names: list[str]) -> JsonValue:
    answer: JsonObject = {}
    for name in names:
        if name in headers:
            answer[name] = headers[name]
        else:
            answer[name] = None
    return answer


def request_value(request: httpx.Request, names: list[str]) -> JsonValue:
    answer: JsonObject = {
        "url": str(request.url),
        "body": request_body(request.content),
        "headers": selected(request.headers, names),
    }
    return answer


def first_request(attempt: Attempt, names: list[str]) -> JsonValue:
    if not attempt.requests:
        return None
    return request_value(attempt.requests[0], names)


def pre_call_value(recorder: Recorder) -> JsonValue:
    answer: JsonObject = {
        "input": recorder.pre_input,
        "body": recorder.pre_body,
        "api_base": recorder.pre_api_base,
        "anthropic_beta": recorder.pre_beta,
    }
    return answer


def post_call_value(recorder: Recorder) -> JsonValue:
    answer: JsonObject = {
        "input": recorder.post_input,
        "original_response": recorder.post_response,
        "body": recorder.post_body,
    }
    return answer


def logged_call_value(recorder: Recorder) -> JsonValue:
    parsed_body: JsonValue = recorder.post_body
    match recorder.post_body:
        case str() as body:
            parsed_body = JSON.validate_json(body)
        case int() | float() | bool() | None | dict() | list():
            pass
    answer: JsonObject = {
        "input": recorder.post_input,
        "body": parsed_body,
    }
    return answer


def reply_value(reply: ReplyView, choice: ChoiceView) -> JsonValue:
    answer: JsonObject = {
        "model": reply.model,
        "content": choice.message.content,
        "finish_reason": choice.finish_reason,
    }
    return answer


def reply_answer(attempt: Attempt) -> JsonValue:
    reply = ReplyView.model_validate(result_of(attempt))
    answer: JsonObject = {
        "reply": reply_value(reply, reply.choices[0]),
        "post_call": post_call_value(attempt.recorder),
    }
    return answer


def stream_value(wrapper: StreamView) -> JsonObject:
    options = OptionsView.model_validate(wrapper.completion_stream)
    reverse: JsonObject = {}
    if options.tool_name_reverse_map is not None:
        for name, target in options.tool_name_reverse_map.items():
            reverse[name] = target
    settings: JsonObject = {
        "json_mode": options.json_mode,
        "speed": options.speed,
        "tool_name_reverse_map": reverse,
    }
    answer: JsonObject = {
        "model": wrapper.model,
        "provider": wrapper.custom_llm_provider,
        "options": settings,
    }
    return answer


def stream_answer(attempt: Attempt, names: list[str]) -> JsonValue:
    stream = stream_value(StreamView.model_validate(result_of(attempt)))
    answer: JsonObject = {
        "stream": stream,
        "pre_call": pre_call_value(attempt.recorder),
        "request": first_request(attempt, names),
        "post_call": logged_call_value(attempt.recorder),
    }
    return answer
