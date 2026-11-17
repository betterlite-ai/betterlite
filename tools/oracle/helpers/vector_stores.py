import asyncio
import logging
from collections.abc import Callable, Coroutine
from typing import Final, Literal, TypeAlias, TypedDict, cast

from litellm.integrations.vector_store_integrations.vector_store_pre_call_hook import (
    VectorStorePreCallHook,
)
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Object: TypeAlias = dict[str, JsonValue]
Answer: TypeAlias = ModelResponse | ResponsesAPIResponse | ModelResponseStream

# LiteLLM logs the hook it enters before deciding anything; that trace names a Python method, so the
# projection keeps only what the hooks decide.
ENTRY_TRACE: Final = "VectorStorePreCallHook."
RESPONSES_OBJECT: Final = "response"

CallHook: TypeAlias = Callable[[Object, Answer, None], Coroutine[None, None, Answer | None]]


class Call(TypedDict):
    operation: Literal["success", "streaming"]
    request_data: Object
    response: Object


class Target(TypedDict, total=False):
    provider_specific_fields: JsonValue


class Choice(TypedDict, total=False):
    message: Target
    delta: Target


class LoggingShape(TypedDict, total=False):
    model_call_details: Object


RECORD: Final = TypeAdapter(Object)
CHOICES: Final = TypeAdapter(list[Choice])
LOGGING: Final = TypeAdapter(LoggingShape)


class CallLog(dict[str, JsonValue]):
    # a case carries LiteLLM's Logging object as the JSON it holds, and the hook reads one attribute off it
    def __init__(self, logging_object: LoggingShape) -> None:
        details = logging_object.get("model_call_details")
        if details is None:
            details = {}
        super().__init__(model_call_details=details)
        self.model_call_details: Object = details


class Lines(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(ENTRY_TRACE):
            self.lines.append(message)


def run(input_value: Call) -> Object:
    handler = Lines()
    logger = logging.getLogger("LiteLLM")
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    # LiteLLM's own handler writes sub-WARNING records to stdout, where the oracle protocol runs
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(handler)
    logger.setLevel(logging.DEBUG)
    try:
        if input_value["operation"] == "success":
            returned = run_success(input_value, handler)
        else:
            returned = run_streaming(input_value, handler)
    finally:
        logger.removeHandler(handler)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    return returned


def run_success(input_value: Call, handler: Lines) -> Object:
    hook = VectorStorePreCallHook()
    call = cast(CallHook, hook.async_post_call_success_deployment_hook)  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/vector_store_integrations/vector_store_pre_call_hook.py#L415-L470
    response = input_value["response"]
    answer: Answer
    if response.get("object") == RESPONSES_OBJECT:
        answer = ResponsesAPIResponse.model_validate(response)
    else:
        answer = ModelResponse.model_validate(response)
    return outcome(asyncio.run(call(hook_request(input_value["request_data"]), answer, None)), handler)


def run_streaming(input_value: Call, handler: Lines) -> Object:
    hook = VectorStorePreCallHook()
    call = cast(CallHook, hook.async_post_call_streaming_deployment_hook)  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/vector_store_integrations/vector_store_pre_call_hook.py#L472-L512
    chunk = ModelResponseStream.model_validate(input_value["response"])
    return outcome(asyncio.run(call(input_value["request_data"], chunk, None)), handler)


def hook_request(request_data: Object) -> Object:
    logging_object = request_data.get("litellm_logging_obj")
    if logging_object is None:
        return request_data
    request = dict(request_data)
    request["litellm_logging_obj"] = CallLog(LOGGING.validate_python(logging_object))
    return request


def outcome(result: Answer | None, handler: Lines) -> Object:
    if result is None:
        return {"returned": None, "diagnostics": handler.lines}
    return {
        "returned": project(RECORD.validate_json(result.model_dump_json())),
        "diagnostics": handler.lines,
    }


def project(record: Object) -> Object:
    return {
        "id": record.get("id"),
        "object": record.get("object"),
        "output": record.get("output"),
        "vector_store_search_failures": record.get("vector_store_search_failures"),
        "search_results": record.get("search_results"),
        "choices": [project_choice(choice) for choice in choices_of(record)],
    }


def choices_of(record: Object) -> list[Choice]:
    choices = record.get("choices")
    if choices is None:
        return []
    return CHOICES.validate_python(choices)


def project_choice(choice: Choice) -> Object:
    message: JsonValue = None
    delta: JsonValue = None
    if "message" in choice and "provider_specific_fields" in choice["message"]:
        message = choice["message"]["provider_specific_fields"]
    if "delta" in choice and "provider_specific_fields" in choice["delta"]:
        delta = choice["delta"]["provider_specific_fields"]
    return {"message": message, "delta": delta}
