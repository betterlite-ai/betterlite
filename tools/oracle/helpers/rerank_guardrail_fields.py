import asyncio
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.cohere.rerank.guardrail_translation.handler import CohereRerankHandler
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter


class RerankHandler(Protocol):
    async def process_input_messages(
        self, data: dict[str, JsonValue], guardrail_to_apply: CustomGuardrail
    ) -> None: ...

    async def process_output_response(
        self, response: RerankResponse, guardrail_to_apply: CustomGuardrail
    ) -> RerankResponse: ...


HANDLER: RerankHandler = CohereRerankHandler()


# the handler only calls apply_guardrail, so this stands in for a real guardrail's check
class ScriptedCheck:
    def __init__(self, texts: list[JsonValue] | None) -> None:
        self.texts = texts
        self.calls: list[JsonValue] = []

    async def __call__(
        self,
        *,
        inputs: dict[str, JsonValue],
        request_data: dict[str, JsonValue],
        input_type: str,
        logging_obj: Logging | None,
    ) -> dict[str, JsonValue]:
        del logging_obj
        request_data["guardrail_seen"] = True
        self.calls.append(
            {
                "texts": inputs.get("texts"),
                "model": inputs.get("model"),
                "input_type": input_type,
                "request_data": dict(request_data),
            }
        )
        if self.texts is None:
            return inputs
        return {**inputs, "texts": self.texts}


class InputCall(TypedDict):
    operation: Literal["input"]
    data: dict[str, JsonValue]
    texts: NotRequired[list[JsonValue] | None]


class OutputCall(TypedDict):
    operation: Literal["output"]
    output: dict[str, JsonValue]


class Input(TypedDict):
    calls: list[InputCall | OutputCall]


# a method cannot be assigned on an instance under mypy, so the scripted check is set through object
async def drive(call: InputCall | OutputCall) -> JsonValue:
    guardrail = CustomGuardrail(guardrail_name="scripted")
    if call["operation"] == "input":
        check = ScriptedCheck(call.get("texts"))
        data = dict(call["data"])
        object.__setattr__(guardrail, "apply_guardrail", check)
        await HANDLER.process_input_messages(data, guardrail)
        return {"data": data, "calls": check.calls}
    check = ScriptedCheck(None)
    response = TypeAdapter(RerankResponse).validate_python(call["output"])
    object.__setattr__(guardrail, "apply_guardrail", check)
    returned = await HANDLER.process_output_response(response, guardrail)
    return {"same": returned is response, "calls": check.calls}


def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return [runner.run(drive(call)) for call in input_value["calls"]]
