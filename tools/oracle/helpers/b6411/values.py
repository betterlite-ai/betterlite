import litellm
from litellm.llms.sail.common_utils import (
    chat_request_for_sail,
    completion_window_for_service_tier,
    extra_body_for_sail,
    json_body,
    responses_params_with_completion_window,
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


def window_value(step: WindowStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    try:
        window = completion_window_for_service_tier(
            step["service_tier"], model=model, drop_params=step["drop_params"]
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": window, "status": None}


def chat_request_value(step: ChatRequestStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    try:
        request = chat_request_for_sail(
            step["request"], model=model, drop_params=step["drop_params"]
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": OBJECT.validate_python(json_body(request)), "status": None}


def extra_body_value(step: ExtraBodyStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    try:
        extra_body = extra_body_for_sail(
            step["extra_body"],
            step["request_metadata"],
            model=model,
            drop_params=step["drop_params"],
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": OBJECT.validate_python(json_body(extra_body)), "status": None}


def responses_params_value(step: ResponsesParamsStep, model: str) -> JsonValue:
    litellm.drop_params = step["global_drop_params"]
    try:
        params = responses_params_with_completion_window(
            step["params"], model=model, drop_params=step["drop_params"]
        )
    except UNSUPPORTED as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return {"ok": False, "value": None, "status": failure.status_code}
    return {"ok": True, "value": OBJECT.validate_python(json_body(params)), "status": None}


def supported_value(step: SupportedStep) -> JsonValue:
    litellm.open_ai_chat_completion_models = set(step["chat_models"])
    litellm.open_ai_text_completion_models = set(step["text_models"])
    supported: list[JsonValue] = [*CHAT.get_supported_openai_params(step["model"])]
    return {"ok": True, "value": supported, "status": None}
