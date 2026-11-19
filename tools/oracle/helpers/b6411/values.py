from collections.abc import Callable, Mapping
from functools import partial
from typing import TypeVar

import litellm
from litellm.llms.sail.common_utils import (
    CompletionWindow,
    chat_request_for_sail,
    completion_window_for_service_tier as window_for_tier,
    extra_body_for_sail,
    json_body,
    responses_params_with_completion_window as responses_window_params,
)
from pydantic import JsonValue

from .contracts import (
    CHAT,
    OBJECT,
    UNSUPPORTED,
    ChatRequestStep,
    ExtraBodyStep,
    Failure,
    ResponsesParamsStep,
    SupportedStep,
    WindowStep,
)

_Body = TypeVar("_Body")


def failure_record(error: BaseException) -> JsonValue:
    failure = Failure.model_validate(error, from_attributes=True)
    return {"ok": False, "value": None, "status": failure.status_code}


def window_record(call: Callable[[], CompletionWindow | None]) -> JsonValue:
    try:
        window = call()
    except UNSUPPORTED as error:
        return failure_record(error)
    return {"ok": True, "value": window, "status": None}


def body_record(call: Callable[[], Mapping[str, _Body]]) -> JsonValue:
    try:
        body = call()
    except UNSUPPORTED as error:
        return failure_record(error)
    return {"ok": True, "value": OBJECT.validate_python(json_body(body)), "status": None}


def window_value(step: WindowStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    tier = step["service_tier"]
    call = partial(window_for_tier, tier, model=model, drop_params=step["drop_params"])
    return window_record(call)


def chat_request_value(step: ChatRequestStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    request = step["request"]
    call = partial(chat_request_for_sail, request, model=model, drop_params=step["drop_params"])
    return body_record(call)


def extra_body_value(step: ExtraBodyStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    extra_body = step["extra_body"]
    metadata = step["request_metadata"]
    drop_flag = step["drop_params"]
    call = partial(extra_body_for_sail, extra_body, metadata, model=model, drop_params=drop_flag)
    return body_record(call)


def responses_params_value(step: ResponsesParamsStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    params = step["params"]
    call = partial(responses_window_params, params, model=model, drop_params=step["drop_params"])
    return body_record(call)


def supported_value(step: SupportedStep) -> JsonValue:
    litellm.open_ai_chat_completion_models = set(step["chat_models"])
    litellm.open_ai_text_completion_models = set(step["text_models"])
    supported: list[JsonValue] = [*CHAT.get_supported_openai_params(step["model"])]
    return {"ok": True, "value": supported, "status": None}
