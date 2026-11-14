import asyncio
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.llms.cohere.rerank.guardrail_translation.handler import CohereRerankHandler
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter

Fields = dict[str, JsonValue]


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
    self: Scripted, inputs: Fields, request_data: Fields, input_type: str, **_call: JsonValue
) -> Fields:
    request_data["guardrail_seen"] = True
    self.calls.append(
        {
            "texts": inputs.get("texts"),
            "model": inputs.get("model"),
            "input_type": input_type,
            "request_data": dict(request_data),
        }
    )
    return inputs if self.texts is None else {**inputs, "texts": self.texts}


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
    members.update(apply_guardrail=scripted_apply, texts=texts, calls=calls)
    members["records_own_guardrail_information"] = True
    return type("Scripted", (CustomGuardrail,), members)(guardrail_name="scripted")  # type: ignore[no-any-return,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L180


async def drive(call: InputCall | OutputCall) -> JsonValue:
    calls: list[JsonValue] = []
    if call["operation"] == "input":
        guardrail = scripted_guardrail(call.get("texts"), calls)
        await HANDLER.process_input_messages(call["data"], guardrail)
        return {"data": call["data"], "calls": calls}
    guardrail = scripted_guardrail(None, calls)
    response = TypeAdapter(RerankResponse).validate_python(call["output"])
    returned = await HANDLER.process_output_response(response, guardrail)
    return {"same": returned is response, "calls": calls}


def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return [runner.run(drive(call)) for call in input_value["calls"]]
