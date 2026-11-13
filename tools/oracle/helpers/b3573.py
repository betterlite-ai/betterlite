import asyncio
import datetime as dt
from typing import Final, Literal, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.pass_through.guardrail_translation.handler import (
    PassThroughEndpointHandler,
)
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON: Final = TypeAdapter(JsonValue)
USER_ID: Final = TypeAdapter(str | None)
HANDLER: Final = PassThroughEndpointHandler()
HANDLER_SOURCE: Final = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/pass_through/guardrail_translation/handler.py"


class LoggingSpec(TypedDict):
    config: dict[str, JsonValue] | None


class SettingsCall(TypedDict):
    function: Literal["settings"]
    guardrail_name: str | None
    logging: LoggingSpec | None


class ExtractCall(TypedDict):
    function: Literal["extract"]
    data: dict[str, JsonValue]
    field_expressions: list[str] | None


class InputCall(TypedDict):
    function: Literal["input"]
    guardrail_name: str | None
    logging: LoggingSpec | None
    data: dict[str, JsonValue]


class OutputCall(TypedDict):
    function: Literal["output"]
    guardrail_name: str | None
    logging: LoggingSpec | None
    response: JsonValue
    request_data: dict[str, JsonValue] | None
    user_api_key: dict[str, JsonValue] | None


Call = SettingsCall | ExtractCall | InputCall | OutputCall


class Input(TypedDict):
    scenarios: list[Call]


def normalised_text(text: str) -> JsonValue:
    try:
        return JSON.validate_json(text)
    except ValidationError:
        return text


def build_logging(spec: LoggingSpec | None) -> Logging | None:
    if spec is None:
        return None
    logging_obj = Logging(
        model="passthrough",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id="passthrough-guardrail",
        function_id="passthrough-guardrail",
    )
    config = spec["config"]
    if config is not None:
        logging_obj.passthrough_guardrails_config = {}
        logging_obj.passthrough_guardrails_config.update(config)
    return logging_obj


def build_user_key(user_api_key: dict[str, JsonValue] | None) -> UserAPIKeyAuth | None:
    if user_api_key is None:
        return None
    auth = UserAPIKeyAuth()
    auth.user_id = USER_ID.validate_python(user_api_key.get("user_id"))
    return auth


def settings_answer(call: SettingsCall) -> JsonValue:
    settings = HANDLER._get_guardrail_settings(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/pass_through/guardrail_translation/handler.py#L32
        build_logging(call["logging"]), call["guardrail_name"]
    )
    if settings is None:
        return None
    return JSON.validate_json(settings.model_dump_json())


def extract_answer(call: ExtractCall) -> JsonValue:
    text = HANDLER._extract_text_for_guardrail(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/pass_through/guardrail_translation/handler.py#L53
        data=call["data"], field_expressions=call["field_expressions"]
    )
    return normalised_text(text)


async def input_answer(call: InputCall) -> JsonValue:
    returned = await HANDLER.process_input_messages(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/pass_through/guardrail_translation/handler.py#L88
        data=call["data"],
        guardrail_to_apply=CustomGuardrail(guardrail_name=call["guardrail_name"]),
        litellm_logging_obj=build_logging(call["logging"]),
    )
    return {"returned": JSON.validate_python(returned)}


async def output_answer(call: OutputCall) -> JsonValue:
    request_data = call["request_data"]
    returned = await HANDLER.process_output_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/pass_through/guardrail_translation/handler.py#L129
        response=call["response"],
        guardrail_to_apply=CustomGuardrail(guardrail_name=call["guardrail_name"]),
        litellm_logging_obj=build_logging(call["logging"]),
        user_api_key_dict=build_user_key(call["user_api_key"]),
        request_data=request_data,
    )
    answer: dict[str, JsonValue] = {"returned": JSON.validate_python(returned)}
    if request_data is not None:
        answer["request_data"] = request_data
    return answer


async def answer(call: Call) -> JsonValue:
    if call["function"] == "settings":
        return settings_answer(call)
    if call["function"] == "extract":
        return extract_answer(call)
    if call["function"] == "input":
        return await input_answer(call)
    return await output_answer(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [
        asyncio.run(answer(scenario)) for scenario in input_value["scenarios"]
    ]
    return answers
