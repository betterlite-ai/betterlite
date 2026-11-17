import datetime as dt
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
    extras = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
        dict[str, JsonValue] | None,
        response.model_extra,
    )
    if extras is None:
        return {}
    return {key: value for key, value in extras.items() if key != USAGE_KEY}


def _hidden(response: ModelResponse) -> dict[str, JsonValue]:
    hidden_params: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
        dict[str, JsonValue],
        response._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    )
    return EXTRAS.validate_python(hidden_params)


def _record(response: ModelResponse) -> dict[str, JsonValue]:
    view = ENVELOPE.validate_json(response.model_dump_json())
    record: dict[str, JsonValue] = {}
    record["id"] = view["id"]
    record["created"] = view["created"]
    record["model"] = view["model"]
    record["object"] = view["object"]
    record["system_fingerprint"] = view["system_fingerprint"]
    record["choices"] = [
        {
            "finish_reason": choice["finish_reason"],
            "index": choice["index"],
            "message": _message(choice["message"]),
        }
        for choice in view["choices"]
    ]
    record["usage"] = view["usage"]
    record.update(_provider(response))
    return record


def _logging(recorded: str | None) -> Logging:
    messages: list[JsonValue] = []
    logging_object = Logging(
        model="caller-model",
        messages=messages,
        stream=False,
        call_type="completion",
        start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    if recorded is not None:
        logging_object.model_call_details[RECORDING_KEY] = recorded  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L707
    return logging_object


def _caller(input_value: Input) -> ModelResponse:
    seed: dict[str, JsonValue] = {}
    seed["created"] = CALLER_CREATED
    seed["object"] = CHAT_OBJECT
    seed["system_fingerprint"] = CALLER_FINGERPRINT
    seed.update(input_value["caller_extra"])
    result = ModelResponse.model_validate(seed)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181
    result._hidden_params = input_value["caller_hidden_params"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    return result


def _transform(input_value: Input) -> ModelResponse:
    raw = RESPONSE.validate_json(json.dumps(input_value["response"]))
    raw._hidden_params = input_value["raw_hidden_params"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1424
    request_data: dict[str, JsonValue] = {}
    optional_params: dict[str, JsonValue] = {}
    litellm_params: dict[str, JsonValue] = {}
    return LiteLLMResponsesTransformationHandler().transform_response(  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L286
        model=input_value["model"],
        raw_response=raw,
        model_response=_caller(input_value),
        logging_obj=_logging(input_value["recorded"]),
        request_data=request_data,
        messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L897
        optional_params=optional_params,
        litellm_params=litellm_params,
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
