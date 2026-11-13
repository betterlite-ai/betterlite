import asyncio
from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.integrations import custom_guardrail
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import GuardrailStatus
from pydantic import JsonValue as Json
from pydantic import TypeAdapter


class RecordInput(TypedDict):
    function: Literal["record"]
    guardrail_name: str | None
    event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | None
    event_type: GuardrailEventHooks | None
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


class Hook(Protocol):
    async def __call__(
        self,
        guardrail: CustomGuardrail,
        *,
        request_data: dict[str, Json],
        inputs: dict[str, Json],
        response: Json,
    ) -> Json: ...


class LoggingStub:
    def __init__(
        self,
        litellm_params: dict[str, Json] | None,
        model_call_params: dict[str, Json] | None,
    ) -> None:
        self.litellm_params = litellm_params
        self.model_call_details: dict[str, Json] = {"litellm_params": model_call_params}


def record(input_value: RecordInput) -> Json:
    args = TypeAdapter(RecordInput).validate_python(input_value)
    guardrail = CustomGuardrail(
        guardrail_name=args["guardrail_name"], event_hook=args["event_hook"]
    )
    request_data: dict[str, Json | LoggingStub] = {
        **args["request_data"],
        "litellm_logging_obj": LoggingStub(args["litellm_params"], args["model_call_params"]),
    }
    guardrail.add_standard_logging_guardrail_information_to_request_data(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1209
        guardrail_json_response=args["guardrail_response"],
        request_data=request_data,
        guardrail_status=args["guardrail_status"],
        start_time=args["start_time"],
        end_time=args["end_time"],
        duration=args["duration"],
        masked_entity_count=args["masked_entity_count"],
        guardrail_provider=args["guardrail_provider"],
        event_type=args["event_type"],
    )
    del request_data["litellm_logging_obj"]
    return {
        "request_data": TypeAdapter(dict[str, Json]).validate_python(request_data),
        "litellm_params": args["litellm_params"],
        "model_call_params": args["model_call_params"],
    }


async def async_pre_call_hook(
    guardrail: CustomGuardrail,
    *,
    request_data: dict[str, Json],
    inputs: dict[str, Json],
    response: Json,
) -> Json:
    del guardrail, request_data, inputs
    return response


async def async_post_call_success_hook(
    guardrail: CustomGuardrail,
    *,
    request_data: dict[str, Json],
    inputs: dict[str, Json],
    response: Json,
) -> Json:
    del guardrail, request_data, inputs
    return response


async def apply_guardrail(
    guardrail: CustomGuardrail,
    *,
    request_data: dict[str, Json],
    inputs: dict[str, Json],
    response: Json,
) -> Json:
    del guardrail, request_data, inputs
    return response


HOOKS: dict[str, Hook] = {
    "pre_call": async_pre_call_hook,
    "post_call": async_post_call_success_hook,
    "apply_guardrail": apply_guardrail,
}


def summarize(input_value: SummarizeInput) -> Json:
    guardrail = CustomGuardrail(guardrail_name="probe")
    request_data = input_value["request_data"]
    wrapped: Hook = custom_guardrail.log_guardrail_information(HOOKS[input_value["hook"]])  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1580
    asyncio.run(
        wrapped(
            guardrail,
            request_data=request_data,
            inputs=input_value["inputs"],
            response=input_value["response"],
        )
    )
    return {"request_data": request_data}


def run(input_value: Input) -> Json:
    if input_value["function"] == "record":
        return record(input_value)
    return summarize(input_value)
