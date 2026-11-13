import asyncio
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

from litellm.integrations.custom_guardrail import (
    CustomGuardrail,
    log_guardrail_information,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580
)
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import GuardrailStatus

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RecordInput(TypedDict):
    function: Literal["record"]
    guardrail_name: str | None
    event_hook: str | list[str] | None
    event_type: str | None
    guardrail_response: Json
    guardrail_status: GuardrailStatus
    start_time: float | None
    end_time: float | None
    duration: float | None
    masked_entity_count: dict[str, int] | None
    guardrail_provider: str | None
    request_data: dict[str, Json]
    litellm_params: dict[str, Json] | None
    model_call_params: dict[str, Json] | None


class SummarizeInput(TypedDict):
    function: Literal["summarize"]
    hook: Literal["pre_call", "post_call", "apply_guardrail"]
    request_data: dict[str, Json]
    inputs: dict[str, Json]
    response: Json


Input: TypeAlias = RecordInput | SummarizeInput


class LoggingStub:
    def __init__(
        self,
        litellm_params: dict[str, Json] | None,
        model_call_params: dict[str, Json] | None,
    ) -> None:
        self.litellm_params = litellm_params
        self.model_call_details: dict[str, Json] = {"litellm_params": model_call_params}


class DataHook(Protocol):
    async def __call__(
        self, guardrail: CustomGuardrail, *, data: dict[str, Json], response: Json
    ) -> Json: ...


class ApplyHook(Protocol):
    async def __call__(
        self,
        guardrail: CustomGuardrail,
        *,
        inputs: dict[str, Json],
        request_data: dict[str, Json],
        response: Json,
    ) -> Json: ...


def configured_hook(
    event_hook: str | list[str] | None,
) -> GuardrailEventHooks | list[GuardrailEventHooks] | None:
    match event_hook:
        case None:
            return None
        case str():
            return GuardrailEventHooks(event_hook)
        case list():
            return [GuardrailEventHooks(name) for name in event_hook]


def record(input_value: RecordInput) -> Json:
    guardrail = CustomGuardrail(
        guardrail_name=input_value["guardrail_name"],
        event_hook=configured_hook(input_value["event_hook"]),
    )
    event_type = input_value["event_type"]
    logging_obj = LoggingStub(input_value["litellm_params"], input_value["model_call_params"])
    request_data = input_value["request_data"]
    request_data["litellm_logging_obj"] = (
        cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1298
            Json, logging_obj
        )
    )
    guardrail.add_standard_logging_guardrail_information_to_request_data(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1212
        guardrail_json_response=input_value["guardrail_response"],
        request_data=request_data,
        guardrail_status=input_value["guardrail_status"],
        start_time=input_value["start_time"],
        end_time=input_value["end_time"],
        duration=input_value["duration"],
        masked_entity_count=input_value["masked_entity_count"],
        guardrail_provider=input_value["guardrail_provider"],
        event_type=None if event_type is None else GuardrailEventHooks(event_type),
    )
    del request_data["litellm_logging_obj"]
    return {
        "request_data": request_data,
        "litellm_params": input_value["litellm_params"],
        "model_call_params": input_value["model_call_params"],
    }


async def async_pre_call_hook(
    guardrail: CustomGuardrail, *, data: dict[str, Json], response: Json
) -> Json:
    del guardrail, data
    return response


async def async_post_call_success_hook(
    guardrail: CustomGuardrail, *, data: dict[str, Json], response: Json
) -> Json:
    del guardrail, data
    return response


async def apply_guardrail(
    guardrail: CustomGuardrail,
    *,
    inputs: dict[str, Json],
    request_data: dict[str, Json],
    response: Json,
) -> Json:
    del guardrail, inputs, request_data
    return response


def summarize(input_value: SummarizeInput) -> Json:
    guardrail = CustomGuardrail(guardrail_name="probe")
    request_data = input_value["request_data"]
    response = input_value["response"]
    if input_value["hook"] == "apply_guardrail":
        apply_wrapped = cast(ApplyHook, log_guardrail_information(apply_guardrail))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580
        asyncio.run(
            apply_wrapped(
                guardrail,
                inputs=input_value["inputs"],
                request_data=request_data,
                response=response,
            )
        )
        return {"request_data": request_data}
    hook = async_pre_call_hook
    if input_value["hook"] == "post_call":
        hook = async_post_call_success_hook
    data_wrapped = cast(DataHook, log_guardrail_information(hook))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580
    asyncio.run(data_wrapped(guardrail, data=request_data, response=response))
    return {"request_data": request_data}


def run(input_value: Input) -> Json:
    if input_value["function"] == "record":
        return record(input_value)
    return summarize(input_value)
