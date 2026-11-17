import datetime
import json
import logging
from logging.handlers import MemoryHandler
from typing import Final, NotRequired, TypedDict, cast

import litellm
from litellm.completion_extras.litellm_responses_transformation.transformation import (
    LiteLLMResponsesTransformationHandler,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

RECOVERY_MESSAGE: Final = "Recovered empty Responses API output from raw SSE"
RECOVERY_LABEL: Final = "recovered_from_sse"
CALLER_CREATED: Final = 5
CALLER_FINGERPRINT: Final = "fp"
RECORDING_KEY: Final = "original_response"
CHAT_OBJECT: Final = "chat.completion"
USAGE_KEY: Final = "usage"


class Input(TypedDict):
    model: str
    response: JsonValue
    caller_extra: dict[str, JsonValue]
    caller_hidden_params: dict[str, JsonValue]
    raw_hidden_params: dict[str, JsonValue]
    recorded: str | None


class MessageView(TypedDict):
    content: str | None
    function_call: JsonValue
    provider_specific_fields: JsonValue
    role: str
    tool_calls: JsonValue
    annotations: NotRequired[JsonValue]
    reasoning_content: NotRequired[str]
    reasoning_items: NotRequired[JsonValue]


class ChoiceView(TypedDict):
    finish_reason: str
    index: int
    message: MessageView


class EnvelopeView(TypedDict):
    id: str
    created: int
    model: str | None
    object: str
    system_fingerprint: str | None
    choices: list[ChoiceView]
    usage: JsonValue


RESPONSE: Final = TypeAdapter(ResponsesAPIResponse)
ENVELOPE: Final = TypeAdapter(EnvelopeView)
EXTRAS: Final = TypeAdapter(dict[str, JsonValue])


def _message(message: MessageView) -> dict[str, JsonValue]:
    projected: dict[str, JsonValue] = {
        "content": message["content"],
        "function_call": message["function_call"],
        "provider_specific_fields": message["provider_specific_fields"],
        "role": message["role"],
        "tool_calls": message["tool_calls"],
    }
    if "annotations" in message:
        projected["annotations"] = message["annotations"]
    if "reasoning_content" in message:
        projected["reasoning_content"] = message["reasoning_content"]
    if "reasoning_items" in message:
        projected["reasoning_items"] = message["reasoning_items"]
    return projected


def _provider(response: ModelResponse) -> dict[str, JsonValue]:
    extras = response.model_extra
    if extras is None:
        return {}
    fields = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
        dict[str, JsonValue],
        extras,
    )
    return {key: value for key, value in fields.items() if key != USAGE_KEY}


def _hidden(response: ModelResponse) -> dict[str, JsonValue]:
    hidden_params: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
        dict[str, JsonValue],
        response._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    )
    return EXTRAS.validate_python(hidden_params)


def _record(response: ModelResponse) -> dict[str, JsonValue]:
    view = ENVELOPE.validate_json(response.model_dump_json())
    record: dict[str, JsonValue] = {
        "id": view["id"],
        "created": view["created"],
        "model": view["model"],
        "object": view["object"],
        "system_fingerprint": view["system_fingerprint"],
        "choices": [
            {
                "finish_reason": choice["finish_reason"],
                "index": choice["index"],
                "message": _message(choice["message"]),
            }
            for choice in view["choices"]
        ],
        "usage": view["usage"],
    }
    record.update(_provider(response))
    return record


def _logging(recorded: str | None) -> Logging:
    logging_object = Logging(
        model="caller-model",
        messages=[],
        stream=False,
        call_type="completion",
        start_time=datetime.datetime(2026, 1, 1, tzinfo=datetime.UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    if recorded is not None:
        logging_object.model_call_details[RECORDING_KEY] = recorded
    return logging_object


def _caller(input_value: Input) -> ModelResponse:
    seed: dict[str, JsonValue] = {
        "created": CALLER_CREATED,
        "object": CHAT_OBJECT,
        "system_fingerprint": CALLER_FINGERPRINT,
    }
    seed.update(input_value["caller_extra"])
    result = ModelResponse.model_validate(seed)
    hidden = input_value["caller_hidden_params"]
    result._hidden_params = hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    return result


def _transform(input_value: Input) -> ModelResponse:
    raw = RESPONSE.validate_json(json.dumps(input_value["response"]))
    hidden = input_value["raw_hidden_params"]
    raw._hidden_params = hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1424
    return LiteLLMResponsesTransformationHandler().transform_response(
        model=input_value["model"],
        raw_response=raw,
        model_response=_caller(input_value),
        logging_obj=_logging(input_value["recorded"]),
        request_data={},
        messages=[],
        optional_params={},
        litellm_params={},
        encoding=None,
    )


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        response = _transform(input_value)
    finally:
        logger.removeHandler(capture)
    return {
        "value": {
            "record": _record(response),
            "hidden_params": _hidden(response),
        },
        "warnings": [
            RECOVERY_LABEL
            for entry in capture.buffer
            if RECOVERY_MESSAGE in entry.getMessage()
        ],
    }
