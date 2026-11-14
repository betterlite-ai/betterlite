import asyncio
import datetime as dt
from collections.abc import Collection
from typing import Final, Literal, NotRequired, Protocol, TypedDict, TypeVar

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.pass_through.guardrail_translation.handler import (
    PassThroughEndpointHandler,
)
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON: Final = TypeAdapter(JsonValue)
USER_ID: Final = TypeAdapter(str | None)
GUARDRAIL_CLASS: Final = TypeAdapter(type[CustomGuardrail])
GUARDRAIL_BASE: Final = type(CustomGuardrail(guardrail_name=None))

Result_co = TypeVar("Result_co", covariant=True, default=object)


class InputView(Protocol):
    async def process_input_messages(
        self,
        data: dict[str, JsonValue],
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
    ) -> Collection[str]: ...


class OutputView(Protocol[Result_co]):
    async def process_output_response(
        self,
        response: JsonValue,
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: UserAPIKeyAuth | None = None,
        request_data: dict[str, JsonValue] | None = None,
    ) -> Result_co: ...


class TextExtraction(Protocol):
    def _extract_text_for_guardrail(
        self, data: dict[str, JsonValue], field_expressions: list[str] | None
    ) -> str: ...

    def text(self, data: dict[str, JsonValue], field_expressions: list[str] | None) -> str:
        return self._extract_text_for_guardrail(data, field_expressions)


class HandlerReader(PassThroughEndpointHandler, TextExtraction):
    def settings(self, logging_obj: Logging | None, guardrail_name: str | None) -> JsonValue:
        settings = self._get_guardrail_settings(logging_obj, guardrail_name)
        if settings is None:
            return None
        return JSON.validate_json(settings.model_dump_json())


class Inputs(TypedDict):
    texts: list[str]
    model: NotRequired[JsonValue]


class Recording(Protocol):
    calls: list[JsonValue]


async def recording_apply(
    self: Recording,
    inputs: Inputs,
    request_data: dict[str, JsonValue],
    input_type: str,
    **_call: JsonValue,
) -> Inputs:
    self.calls.append(
        {
            "texts": [normalised_text(text) for text in inputs["texts"]],
            "model": inputs.get("model"),
            "input_type": input_type,
            "request_data": dict(request_data),
        }
    )
    return inputs


def recording_guardrail(name: str | None, calls: list[JsonValue]) -> CustomGuardrail:
    members = {
        "apply_guardrail": recording_apply,
        "calls": calls,
        "records_own_guardrail_information": True,
    }
    recording_class = type("Recording", (GUARDRAIL_BASE,), members)
    guardrail_class = GUARDRAIL_CLASS.validate_python(recording_class)
    return guardrail_class(guardrail_name=name)


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


async def call_input(
    handler: InputView,
    data: dict[str, JsonValue],
    guardrail: CustomGuardrail,
    logging_obj: Logging | None,
) -> Collection[str]:
    return await handler.process_input_messages(
        data=data,
        guardrail_to_apply=guardrail,
        litellm_logging_obj=logging_obj,
    )


async def input_answer(call: InputCall) -> JsonValue:
    calls: list[JsonValue] = []
    returned = await call_input(
        PassThroughEndpointHandler(),
        call["data"],
        recording_guardrail(call["guardrail_name"], calls),
        build_logging(call["logging"]),
    )
    return {"returned": JSON.validate_python(returned), "calls": calls}


async def output_answer(call: OutputCall, handler: OutputView) -> JsonValue:
    request_data = call["request_data"]
    calls: list[JsonValue] = []
    returned = await handler.process_output_response(
        response=call["response"],
        guardrail_to_apply=recording_guardrail(call["guardrail_name"], calls),
        litellm_logging_obj=build_logging(call["logging"]),
        user_api_key_dict=build_user_key(call["user_api_key"]),
        request_data=request_data,
    )
    answer: dict[str, JsonValue] = {"returned": JSON.validate_python(returned), "calls": calls}
    if request_data is not None:
        answer["request_data"] = request_data
    return answer


async def answer(call: Call) -> JsonValue:
    if call["function"] == "settings":
        return HandlerReader().settings(build_logging(call["logging"]), call["guardrail_name"])
    if call["function"] == "extract":
        return normalised_text(HandlerReader().text(call["data"], call["field_expressions"]))
    if call["function"] == "input":
        return await input_answer(call)
    return await output_answer(call, PassThroughEndpointHandler())


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [
        asyncio.run(answer(scenario)) for scenario in input_value["scenarios"]
    ]
    return answers
