import logging
import time
from dataclasses import dataclass, field
from functools import partial
from typing import Final, NotRequired, Protocol, TypedDict, cast, runtime_checkable

import httpx
import litellm
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from litellm.llms.anthropic.common_utils import AnthropicError
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

ANTHROPIC_FAILURE: Final[type[Exception]] = AnthropicError
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class Input(TypedDict):
    id: str
    status: int
    raw_text: str
    headers: dict[str, str]
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    json_mode: bool | None
    now: int
    disable_add_prefix_to_prompt: bool


class MessageView(TypedDict):
    role: str
    content: str | None
    tool_calls: JsonValue
    thinking_blocks: NotRequired[JsonValue]
    reasoning_content: NotRequired[str | None]
    provider_specific_fields: NotRequired[JsonValue]


class ChoiceView(TypedDict):
    finish_reason: str | None
    message: MessageView


class CompletionDetailsView(TypedDict):
    reasoning_tokens: int | None
    text_tokens: int | None


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cache_creation_input_tokens: int
    cache_read_input_tokens: int
    completion_tokens_details: CompletionDetailsView
    speed: NotRequired[str | None]


class EnvelopeView(TypedDict):
    id: str
    object: str
    created: int
    model: str | None
    choices: list[ChoiceView]
    usage: UsageView


class HiddenView(TypedDict):
    additional_headers: dict[str, JsonValue]
    provider_specific_fields: JsonValue


class AdditionalArgsView(TypedDict):
    complete_input_dict: JsonValue


@runtime_checkable
class LoggingSink(Protocol):
    def post_call(self, **kwargs: JsonValue) -> None: ...


@runtime_checkable
class Transformer(Protocol):
    # the message type carries Any, so the config enters through this JSON-facing signature
    def transform_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/transformation.py#L2709-L2722
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: LoggingSink,
        request_data: dict[str, JsonValue],
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
        *,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


class Failure(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    status_code: int
    headers: dict[str, str]


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.fired = False

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.WARNING:
            self.fired = True


@dataclass
class Logging(LoggingSink):
    kwargs: dict[str, JsonValue] = field(default_factory=dict)

    @override
    def post_call(self, **kwargs: JsonValue) -> None:
        self.kwargs = kwargs


ENVELOPE: Final = TypeAdapter(EnvelopeView)
HIDDEN: Final = TypeAdapter(HiddenView)
ADDITIONAL: Final = TypeAdapter(AdditionalArgsView)
HEADER_FIELDS: Final = TypeAdapter(dict[str, JsonValue])
TRANSFORMER: Final = TypeAdapter[Transformer](Transformer, config=ARBITRARY)


def _transform(input_value: Input, logging_object: Logging) -> ModelResponse:
    transformer = TRANSFORMER.validate_python(AnthropicConfig())
    return transformer.transform_response(
        model="",
        raw_response=httpx.Response(
            input_value["status"],
            content=input_value["raw_text"].encode(),
            headers=input_value["headers"],
        ),
        model_response=ModelResponse(id=input_value["id"]),
        logging_obj=logging_object,
        request_data=input_value["request_data"],
        messages=input_value["messages"],
        optional_params=input_value["optional_params"],
        litellm_params=input_value["litellm_params"],
        encoding=None,
        json_mode=input_value["json_mode"],
    )


def _message(message: MessageView) -> dict[str, JsonValue]:
    return {
        "role": message["role"],
        "content": message["content"],
        "tool_calls": message["tool_calls"],
        "thinking_blocks": message.get("thinking_blocks"),
        "reasoning_content": message.get("reasoning_content"),
        "provider_specific_fields": message.get("provider_specific_fields"),
    }


def _usage(usage: UsageView) -> dict[str, JsonValue]:
    details = usage["completion_tokens_details"]
    return {
        "prompt_tokens": usage["prompt_tokens"],
        "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
        "cache_creation_input_tokens": usage["cache_creation_input_tokens"],
        "cache_read_input_tokens": usage["cache_read_input_tokens"],
        "reasoning_tokens": details["reasoning_tokens"],
        "text_tokens": details["text_tokens"],
        "speed": usage.get("speed"),
    }


def _hidden(response: ModelResponse) -> dict[str, JsonValue]:
    hidden_params: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
        dict[str, JsonValue],
        response._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    )
    hidden = HIDDEN.validate_python(hidden_params)
    return {
        "additional_headers": HEADER_FIELDS.validate_python(hidden["additional_headers"]),
        "provider_specific_fields": hidden["provider_specific_fields"],
    }


def _envelope(response: ModelResponse) -> dict[str, JsonValue]:
    envelope = ENVELOPE.validate_json(response.model_dump_json())
    choice = envelope["choices"][0]
    return {
        "kind": "reply",
        "id": envelope["id"],
        "object": envelope["object"],
        "created": envelope["created"],
        "model": envelope["model"],
        "finish_reason": choice["finish_reason"],
        "message": _message(choice["message"]),
        "usage": _usage(envelope["usage"]),
        "hidden": _hidden(response),
    }


def _logged(logging_object: Logging) -> dict[str, JsonValue]:
    additional = ADDITIONAL.validate_python(logging_object.kwargs.get("additional_args"))
    return {
        "input": logging_object.kwargs.get("input"),
        "original_response": logging_object.kwargs.get("original_response"),
        "complete_input_dict": additional["complete_input_dict"],
    }


def _failure(failure: Failure, logging_object: Logging, *, warned: bool) -> JsonValue:
    headers = HEADER_FIELDS.validate_python(failure.headers)
    outcome: JsonValue = {"kind": "error", "status": failure.status_code, "headers": headers}
    return {"outcome": outcome, "logged": _logged(logging_object), "warned": warned}


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    logging_object = Logging()
    previous = litellm.disable_add_prefix_to_prompt
    litellm.disable_add_prefix_to_prompt = input_value["disable_add_prefix_to_prompt"]
    clock = time.time
    time.time = partial(float, input_value["now"])
    try:
        response = _transform(input_value, logging_object)
    except ANTHROPIC_FAILURE as error:
        failure = Failure.model_validate(error, from_attributes=True)
        return _failure(failure, logging_object, warned=warnings.fired)
    finally:
        time.time = clock
        logger.removeHandler(warnings)
        litellm.disable_add_prefix_to_prompt = previous
    return {
        "outcome": _envelope(response),
        "logged": _logged(logging_object),
        "warned": warnings.fired,
    }
