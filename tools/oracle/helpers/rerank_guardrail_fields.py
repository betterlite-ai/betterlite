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


class Scripted(Protocol):
    texts: list[JsonValue] | None
    calls: list[JsonValue]


async def scripted_apply(
    self: Scripted,
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


OBJECTS = TypeAdapter(dict[str, object])


def scripted_guardrail(texts: list[JsonValue] | None, calls: list[JsonValue]) -> CustomGuardrail:
    members = OBJECTS.validate_json("{}")
    members["apply_guardrail"] = scripted_apply
    members["texts"] = texts
    members["calls"] = calls
    members["records_own_guardrail_information"] = True
    return type("Scripted", (CustomGuardrail,), members)(guardrail_name="scripted")  # type: ignore[no-any-return,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L180


async def drive(call: InputCall | OutputCall) -> JsonValue:
    calls: list[JsonValue] = []
    guardrail = scripted_guardrail(
        call.get("texts") if call["operation"] == "input" else None, calls
    )
    if call["operation"] == "input":
        data = dict(call["data"])
        await HANDLER.process_input_messages(data, guardrail)
        return {"data": data, "calls": calls}
    response = TypeAdapter(RerankResponse).validate_python(call["output"])
    returned = await HANDLER.process_output_response(response, guardrail)
    return {"same": returned is response, "calls": calls}


def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return [runner.run(drive(call)) for call in input_value["calls"]]
