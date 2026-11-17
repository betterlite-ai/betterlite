import datetime as dt
import json
import logging

from logging.handlers import BufferingHandler
from typing import Final, TypedDict

import litellm
from litellm.completion_extras.litellm_responses_transformation.transformation import (
    LiteLLMResponsesTransformationHandler,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(dict[str, JsonValue])
WATCHED: Final = (
    "Chat provider: Stream parameter:",
    "Chat provider: Warning ignoring previous response ID:",
    "Chat provider: Final request model=",
)


class Input(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]
    reasoning_auto_summary: bool
    client: str | None


def request_view(data: dict[str, object]) -> dict[str, JsonValue]:
    del data["litellm_logging_obj"]
    return JSON.validate_json(json.dumps(data))


def captured_diagnostics(capture: BufferingHandler) -> list[JsonValue]:
    texts = [record.getMessage() for record in capture.buffer]
    return [text for text in texts if text.startswith(WATCHED)]


def run(input_value: Input) -> JsonValue:
    handler = LiteLLMResponsesTransformationHandler()
    logging_obj = Logging(
        model=input_value["model"],
        messages=None,
        stream=False,
        call_type="completion",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id="response-chat-request",
        function_id="response-chat-request",
    )
    optional_params = dict(input_value["optional_params"])
    capture = BufferingHandler(capacity=100_000)
    verbose_logger = logging.getLogger("LiteLLM")
    saved_handlers = verbose_logger.handlers[:]
    saved_level = verbose_logger.level
    saved_summary = litellm.reasoning_auto_summary
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    litellm.reasoning_auto_summary = input_value["reasoning_auto_summary"]
    try:
        request_data = handler.transform_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L581
            model=input_value["model"],
            messages=input_value["messages"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L584
            optional_params=optional_params,
            litellm_params=dict(input_value["litellm_params"]),
            headers=dict(input_value["headers"]),
            litellm_logging_obj=logging_obj,
            client=input_value["client"],
        )
        request = request_view(request_data)  # type: ignore[misc]  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L581
        call_type: str = logging_obj.call_type  # type: ignore[misc, assignment]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L618
    finally:
        verbose_logger.handlers = saved_handlers
        verbose_logger.setLevel(saved_level)
        litellm.reasoning_auto_summary = saved_summary
    return {
        "request": request,
        "optional_params": optional_params,
        "call_type": call_type,
        "diagnostics": captured_diagnostics(capture),
    }
