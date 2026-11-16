import asyncio
from typing import Final, Literal, Protocol, TypedDict, TypeVar

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.llms.base_llm.guardrail_translation.utils import UnappliableRequestRewrite
from litellm.llms.openai.responses.guardrail_translation.handler import (
    OpenAIResponsesHandler,
    ResponseOutputEnvelope,
    scannable_instructions,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
GUARDRAIL_CLASS: Final = TypeAdapter(type[CustomGuardrail])
GUARDRAIL_BASE: Final = type(CustomGuardrail(guardrail_name=None))

Result_co = TypeVar("Result_co", covariant=True, default=object)


class CoveringGuardrail(CustomGuardrail):
    def structured_messages_cover_full_request(self) -> bool:
        return True


class HandlerView(Protocol[Result_co]):
    def get_structured_messages(self, data: dict[str, JsonValue]) -> Result_co: ...

    async def process_input_messages(
        self, data: dict[str, JsonValue], guardrail_to_apply: CustomGuardrail
    ) -> Result_co: ...

    async def process_output_response(
        self,
        response: ResponseOutputEnvelope,
        guardrail_to_apply: CustomGuardrail,
        request_data: dict[str, JsonValue] | None = None,
    ) -> Result_co: ...

    async def _apply_guardrail_responses_to_output(
        self,
        response: ResponseOutputEnvelope,
        responses: list[str],
        task_mappings: list[tuple[int, int]],
    ) -> Result_co: ...


class Inputs(TypedDict, total=False):
    texts: list[JsonValue]
    images: list[JsonValue]
    tools: list[JsonValue]
    tool_calls: list[JsonValue]
    structured_messages: list[JsonValue]
    model: str


class Recording(Protocol):
    calls: list[JsonValue]
    reply: Inputs | None


class DataScenario(TypedDict):
    data: dict[str, JsonValue]
    skip_system: bool
    covers_full: bool
    reply: Inputs | None


class OutputScenario(TypedDict):
    response: ResponseOutputEnvelope
    request_data: dict[str, JsonValue] | None
    reply: Inputs | None


class ApplyScenario(TypedDict):
    response: ResponseOutputEnvelope
    responses: list[str]
    task_mappings: list[tuple[int, int]]


class DataCall(TypedDict):
    function: Literal["request_view", "input"]
    scenarios: list[DataScenario]


class OutputCall(TypedDict):
    function: Literal["output"]
    scenarios: list[OutputScenario]


class ApplyCall(TypedDict):
    function: Literal["apply_output"]
    scenarios: list[ApplyScenario]


Input = DataCall | OutputCall | ApplyCall


async def recording_apply(
    self: Recording,
    inputs: Inputs,
    request_data: dict[str, JsonValue],
    input_type: str,
    **_call: JsonValue,
) -> Inputs:
    self.calls.append(
        JSON.validate_python(
            {
                "input_type": input_type,
                "inputs": dict(inputs),
                "request_keys": sorted(request_data),
            }
        )
    )
    if self.reply is None:
        return inputs
    return self.reply


def recording_guardrail(
    calls: list[JsonValue],
    reply: Inputs | None,
    *,
    skip_system: bool,
    covers_full: bool,
) -> CustomGuardrail:
    members = {
        "apply_guardrail": recording_apply,
        "calls": calls,
        "reply": reply,
        "skip_system_message_in_guardrail": skip_system,
    }
    base = CoveringGuardrail if covers_full else GUARDRAIL_BASE
    recording_class = type("Recording", (base,), members)
    guardrail_class = GUARDRAIL_CLASS.validate_python(recording_class)
    return guardrail_class(guardrail_name="guard")


async def input_answer(handler: HandlerView, scenario: DataScenario) -> JsonValue:
    calls: list[JsonValue] = []
    data = scenario["data"]
    guardrail = recording_guardrail(
        calls,
        scenario["reply"],
        skip_system=scenario["skip_system"],
        covers_full=scenario["covers_full"],
    )
    try:
        await handler.process_input_messages(data=data, guardrail_to_apply=guardrail)
    except UnappliableRequestRewrite:
        return {"error": "unappliable_request_rewrite", "data": data, "calls": calls}
    return {"data": data, "calls": calls}


async def output_answer(handler: HandlerView, scenario: OutputScenario) -> JsonValue:
    calls: list[JsonValue] = []
    guardrail = recording_guardrail(
        calls, scenario["reply"], skip_system=False, covers_full=False
    )
    response = scenario["response"]
    await handler.process_output_response(
        response=response,
        guardrail_to_apply=guardrail,
        request_data=scenario["request_data"],
    )
    return {"response": JSON.validate_python(response), "calls": calls}


async def apply_answer(handler: HandlerView, scenario: ApplyScenario) -> JsonValue:
    apply = handler._apply_guardrail_responses_to_output  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/guardrail_translation/handler.py#L1509
    response = scenario["response"]
    await apply(response, scenario["responses"], scenario["task_mappings"])
    return JSON.validate_python(response)


async def answer(call: Input) -> list[JsonValue]:
    handler: HandlerView = OpenAIResponsesHandler()
    if call["function"] == "apply_output":
        return [await apply_answer(handler, item) for item in call["scenarios"]]
    if call["function"] == "output":
        return [await output_answer(handler, item) for item in call["scenarios"]]
    if call["function"] == "request_view":
        return [
            {
                "messages": JSON.validate_python(
                    handler.get_structured_messages(item["data"])
                ),
                "instructions": scannable_instructions(
                    item["data"], skip_system=item["skip_system"]
                ),
            }
            for item in call["scenarios"]
        ]
    return [await input_answer(handler, item) for item in call["scenarios"]]


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = asyncio.run(answer(input_value))
    return answers
