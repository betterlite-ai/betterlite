import time
from datetime import UTC, datetime
from functools import partial

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue

from .contracts import (
    JSON,
    OPENAI_ERROR,
    FailureRecord,
    RecordedLogging,
    ReplyStep,
    config_for,
)
from .values import CLOCK, failure


def reply_value(step: ReplyStep) -> JsonValue:
    time.time = partial(float, CLOCK)
    config = config_for("MAI-Image-2.5")
    logging_obj = Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="image_generation",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    status = step.get("status")
    try:
        response = config.transform_image_edit_response(
            "MAI-Image-2.5",
            httpx.Response(200 if status is None else status, content=step["body"].encode()),
            logging_obj,
        )
    except OPENAI_ERROR as error:
        record = FailureRecord.model_validate(error, from_attributes=True)
        return failure("OpenAIError", record.status_code, None)
    except ValueError:
        return failure("ValueError", None, None)
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    logged: JsonValue = None
    if details.original_response is not None and details.additional_args is not None:
        logged = {
            "input": details.input,
            "api_key": details.api_key,
            "original_response": JSON.validate_json(details.original_response),
            "complete_input_dict": details.additional_args["complete_input_dict"],
        }
    data: list[JsonValue] = []
    entries = response.data
    if entries is not None:
        data = [JSON.validate_json(image.model_dump_json(exclude_none=True)) for image in entries]
    usage = response.usage
    projected_usage: JsonValue = {
        "input_tokens": 0,
        "input_tokens_details": {"image_tokens": 0, "text_tokens": 0},
        "output_tokens": 0,
        "total_tokens": 0,
    }
    if usage is not None:
        # the four counts only, since the response keeps the provider's aliases as extras
        projected_usage = {
            "input_tokens": usage.input_tokens,
            "input_tokens_details": {
                "image_tokens": usage.input_tokens_details.image_tokens,
                "text_tokens": usage.input_tokens_details.text_tokens,
            },
            "output_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
        }
    return {
        "created": response.created,
        "data": data,
        "size": response.size,
        "usage": projected_usage,
        "logged": logged,
    }
