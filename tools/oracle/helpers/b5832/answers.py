from typing import cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import OpenAITextCompletionUserMessage as UserMessage
from litellm.types.llms.openai import ResponseInputParam
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from b5832.records import (
    API_ORIGIN,
    CALL_ID,
    COMPLETION,
    CONFIG,
    DELETED,
    FIXED_START,
    MODEL,
    CompletionStep,
    DeleteStep,
    ExtraBodyStep,
    HeadersStep,
    MapParamsStep,
    Step,
    TransformStep,
    optional_params_of,
)


def supported_answer() -> JsonValue:
    supported: list[JsonValue] = [*COMPLETION.get_supported_openai_params(MODEL)]
    return supported


def capability_answer() -> JsonValue:
    return {
        "provider": CONFIG.custom_llm_provider.value,
        "native_websocket": CONFIG.supports_native_websocket(),
        "native_file_search": CONFIG.supports_native_file_search(),
    }


def map_params_step(step: MapParamsStep) -> JsonValue:
    non_default: dict[str, JsonValue] = {}
    given = step.get("non_default_params")
    if given is not None:
        non_default.update(given)
    return COMPLETION.map_openai_params(
        non_default, optional_params_of(step), MODEL, drop_params=False
    )


def extra_body_step(step: ExtraBodyStep) -> JsonValue:
    return COMPLETION.map_extra_body_params(
        optional_params=optional_params_of(step), model=step["model"]
    )


def completion_step(step: CompletionStep) -> JsonValue:
    messages = [
        UserMessage(role="user", content=message["content"]) for message in step["messages"]
    ]
    try:
        request = COMPLETION.transform_text_completion_request(
            step["model"], messages, optional_params_of(step), {}
        )
    except ValueError:
        return {"raised": None}
    return request


def headers_step(step: HeadersStep) -> JsonValue:
    session = step["session"]
    fields: dict[str, JsonValue] = {
        "api_key": step.get("api_key"),
        "litellm_session_id": session.get("litellm_session_id"),
        "litellm_metadata": session.get("litellm_metadata"),
    }
    params = GenericLiteLLMParams.model_validate(fields)
    try:
        authorized = CONFIG.validate_environment(
            headers=step["headers"], model=MODEL, litellm_params=params
        )
    except ValueError:
        return {"raised": None}
    return authorized


def transform_step(step: TransformStep) -> JsonValue:
    typed_input = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fireworks_ai/responses/transformation.py#L152
        "str | ResponseInputParam", step["input"]
    )
    try:
        body = CONFIG.transform_responses_api_request(
            step["model"], typed_input, optional_params_of(step), GenericLiteLLMParams(), {}
        )
    except ValueError:
        return {"raised": None}
    return body


def delete_step(step: DeleteStep) -> JsonValue:
    request = httpx.Request("DELETE", API_ORIGIN + step["path"])
    logging_obj = Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type="responses",
        start_time=FIXED_START,
        litellm_call_id=CALL_ID,
        function_id=CALL_ID,
    )
    deleted = CONFIG.transform_delete_response_api_response(
        httpx.Response(200, request=request), logging_obj
    )
    projected = DELETED.validate_json(deleted.model_dump_json())
    return {
        "id": projected["id"],
        "object": projected["object"],
        "deleted": projected["deleted"],
    }


def step_value(step: Step) -> JsonValue:
    if step["op"] == "supported":
        answer = supported_answer()
    elif step["op"] == "capability":
        answer = capability_answer()
    elif step["op"] == "map_params":
        answer = map_params_step(step)
    elif step["op"] == "extra_body":
        answer = extra_body_step(step)
    elif step["op"] == "completion":
        answer = completion_step(step)
    elif step["op"] == "headers":
        answer = headers_step(step)
    elif step["op"] == "url":
        answer = CONFIG.get_complete_url(step.get("api_base"), {})
    elif step["op"] == "transform":
        answer = transform_step(step)
    else:
        answer = delete_step(step)
    return answer
