import re
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
import litellm.utils
from litellm.exceptions import BadRequestError, InternalServerError, RateLimitError
from litellm.llms.anthropic.pass_through.messages.utils import (
    AnthropicMessagesRequestUtils,
    anthropic_system_to_openai_message,
    get_safeguard_refusal_stop_details,
    openai_chat_refusal_text,
    refusal_stop_details,
    responses_output_refusal_text,
    safeguard_refusal_error,
)
from litellm.llms.anthropic.pass_through.messages.utils import (
    mock_response as anthropic_mock_response,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L180
)
from openai.types.chat import ChatCompletionMessage
from openai.types.responses import ResponseOutputItem
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JsonObject: TypeAlias = dict[str, Json]
SdkError: TypeAlias = BadRequestError | InternalServerError | RateLimitError


class Fixture(TypedDict):
    model_cost: JsonObject
    packaged: JsonObject


class OptionalParamsStep(TypedDict):
    function: Literal["get_requested_anthropic_messages_optional_param"]
    params: JsonObject
    model: str | None
    custom_llm_provider: str | None
    drop_params: bool
    global_drop_params: bool


class SafeguardDetailsStep(TypedDict):
    function: Literal["get_safeguard_refusal_stop_details"]
    response: Json


class StopDetailsStep(TypedDict):
    function: Literal["refusal_stop_details"]
    explanation: str | None


class SafeguardErrorStep(TypedDict):
    function: Literal["safeguard_refusal_error"]
    model: str
    stop_details: JsonObject


class ChatRefusalStep(TypedDict):
    function: Literal["openai_chat_refusal_text"]
    refusal: str | None
    provider_specific_fields: JsonObject | None


class TypedItem(TypedDict):
    kind: Literal["typed"]
    value: JsonObject


class RawItem(TypedDict):
    kind: Literal["raw"]
    value: Json


class ResponsesRefusalStep(TypedDict):
    function: Literal["responses_output_refusal_text"]
    output: list[TypedItem | RawItem]


class SystemStep(TypedDict):
    function: Literal["anthropic_system_to_openai_message"]
    system: Json


class MockStep(TypedDict):
    function: Literal["mock_response"]
    model: str
    mock_response: str | None


Step: TypeAlias = (
    OptionalParamsStep
    | SafeguardDetailsStep
    | StopDetailsStep
    | SafeguardErrorStep
    | ChatRefusalStep
    | ResponsesRefusalStep
    | SystemStep
    | MockStep
)


class Input(TypedDict):
    fixture: Fixture
    steps: list[Step]


class Bundled:
    def __init__(self, model_cost: JsonObject) -> None:
        self.model_cost = model_cost

    def __call__(self) -> JsonObject:
        return self.model_cost


OUTPUT_ITEM = TypeAdapter(ResponseOutputItem)
# the message the exception was built with, without the class prefixes its constructors add
PREFIXES = re.compile(r"^(?:litellm\.\w+: )+")


def failure(error: SdkError) -> JsonObject:
    # openai.APIError declares `type` without a default, so a class that never sets it has none
    try:
        error_type = error.type
    except AttributeError:
        error_type = "None"
    name = type(error).__name__
    projected: JsonObject = {
        "exception": name,
        "status": error.status_code,
        "type": error_type,
        "param": error.param,
        "code": error.code,
        "model": error.model,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
        "provider": error.llm_provider,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
    }
    # the gate message quotes the parameter with Python's repr, which betterlite doesn't copy
    if name == "UnsupportedParamsError":
        return projected
    return {**projected, "message": PREFIXES.sub("", error.message)}


def optional_params(step: OptionalParamsStep) -> Json:
    previous = litellm.drop_params
    litellm.drop_params = step["global_drop_params"]
    try:
        kept = AnthropicMessagesRequestUtils.get_requested_anthropic_messages_optional_param(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L136
            step["params"],
            model=step["model"],
            drop_params=step["drop_params"],
            custom_llm_provider=step["custom_llm_provider"],
        )
    finally:
        litellm.drop_params = previous
    ordered = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L136
        JsonObject, kept
    )
    return {"params": [[key, value] for key, value in ordered.items()]}


def stop_details(step: StopDetailsStep) -> Json:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L37
        Json, refusal_stop_details(step["explanation"])
    )


def chat_refusal(step: ChatRefusalStep) -> Json:
    message = ChatCompletionMessage.model_validate(
        {  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L54
            "role": "assistant",
            "refusal": step["refusal"],
            "provider_specific_fields": step["provider_specific_fields"],
        }
    )
    return openai_chat_refusal_text(message)


def responses_refusal(step: ResponsesRefusalStep) -> Json:
    output: list[ResponseOutputItem | Json] = [
        OUTPUT_ITEM.validate_python(item["value"]) if item["kind"] == "typed" else item["value"]
        for item in step["output"]
    ]
    return responses_output_refusal_text(output)


def mock_message(step: MockStep) -> Json:
    if step["mock_response"] is None:
        response = anthropic_mock_response(model=step["model"], messages=[], max_tokens=1)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L180
    else:
        response = anthropic_mock_response(
            model=step["model"], messages=[], max_tokens=1, mock_response=step["mock_response"]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L180
        )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L180
        Json, response
    )


def answer(step: Step) -> Json:
    result: Json
    try:
        if step["function"] == "get_requested_anthropic_messages_optional_param":
            result = optional_params(step)
        elif step["function"] == "get_safeguard_refusal_stop_details":
            result = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L24
                Json, get_safeguard_refusal_stop_details(step["response"])
            )
        elif step["function"] == "refusal_stop_details":
            result = stop_details(step)
        elif step["function"] == "safeguard_refusal_error":
            refusal = safeguard_refusal_error(step["model"], step["stop_details"])
            result = {"error": failure(refusal)}
        elif step["function"] == "openai_chat_refusal_text":
            result = chat_refusal(step)
        elif step["function"] == "responses_output_refusal_text":
            result = responses_refusal(step)
        elif step["function"] == "anthropic_system_to_openai_message":
            result = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/utils.py#L112
                Json, anthropic_system_to_openai_message(step["system"])
            )
        else:
            result = mock_message(step)
    except (BadRequestError, InternalServerError, RateLimitError) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L219
        result = {"error": failure(error)}
    return result


def run(input_value: Input) -> list[Json]:
    fixture = input_value["fixture"]
    bundled = Bundled(fixture["packaged"])
    previous_cost = litellm.model_cost  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    previous_bundled = litellm.utils._get_bundled_model_cost_map  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
    litellm.model_cost = fixture["model_cost"]
    # no public entry injects the packaged cost map, so the fixture replaces the private reader
    litellm.utils._get_bundled_model_cost_map = bundled  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
    try:
        return [answer(step) for step in input_value["steps"]]
    finally:
        litellm.model_cost = previous_cost
        litellm.utils._get_bundled_model_cost_map = previous_bundled  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8419
