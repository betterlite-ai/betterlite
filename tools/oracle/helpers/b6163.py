import asyncio
import json
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.a2a.chat.guardrail_translation.handler import A2AGuardrailHandler
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter

_MemberValue: TypeAlias = dict[str, object]
_HandlerInput: TypeAlias = list[object]
_Output: TypeAlias = object

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
_RESPONSES: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])
_HANDLER_INPUT: TypeAdapter[_HandlerInput] = TypeAdapter(list[object])
_MEMBERS: TypeAdapter[_MemberValue] = TypeAdapter(dict[str, object])


class Scanned(TypedDict, total=False):
    texts: list[str]
    structured_messages: list[JsonValue]
    model: str


class Scripted(Protocol):
    reply: list[str] | None
    calls: list[JsonValue]


class Handler(Protocol):
    async def process_input_messages(
        self,
        data: dict[str, JsonValue],
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
    ) -> dict[str, JsonValue]: ...

    async def process_output_response(
        self,
        response: JsonValue,
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: UserAPIKeyAuth | None = None,
        request_data: dict[str, JsonValue] | None = None,
    ) -> _Output: ...

    async def process_output_streaming_response(  # ast-grep-ignore: no-long-names
        self,
        responses_so_far: _HandlerInput,
        guardrail_to_apply: CustomGuardrail,
        litellm_logging_obj: Logging | None = None,
        user_api_key_dict: UserAPIKeyAuth | None = None,
        request_data: dict[str, JsonValue] | None = None,
    ) -> _HandlerInput: ...


class InputScenario(TypedDict):
    data: dict[str, JsonValue]
    reply: NotRequired[list[str]]


class OutputScenario(TypedDict):
    response: JsonValue
    request_data: NotRequired[dict[str, JsonValue]]
    reply: NotRequired[list[str]]


class StreamScenario(TypedDict):
    responses: list[JsonValue]
    request_data: NotRequired[dict[str, JsonValue]]
    reply: NotRequired[list[str]]


class InputCall(TypedDict):
    function: Literal["input"]
    scenarios: list[InputScenario]


class OutputCall(TypedDict):
    function: Literal["output"]
    scenarios: list[OutputScenario]


class StreamCall(TypedDict):
    function: Literal["stream"]
    scenarios: list[StreamScenario]


Input: TypeAlias = InputCall | OutputCall | StreamCall


async def scripted_apply(
    self: Scripted,
    inputs: Scanned,
    request_data: dict[str, JsonValue],
    input_type: Literal["request", "response"],
    logging_obj: Logging | None = None,
) -> Scanned:
    del logging_obj
    self.calls.append(
        {
            "input_type": input_type,
            "inputs": _JSON.validate_json(json.dumps(inputs)),
            "request_data": _JSON.validate_json(json.dumps(request_data)),
        }
    )
    reply = self.reply
    if reply is None:
        return inputs
    inputs["texts"] = reply
    return inputs


def scripted_guardrail(reply: list[str] | None, calls: list[JsonValue]) -> CustomGuardrail:
    members = _MEMBERS.validate_json("{}")
    members["reply"] = reply
    members["calls"] = calls
    members["records_own_guardrail_information"] = True
    members["apply_guardrail"] = scripted_apply
    return type("Scripted", (CustomGuardrail,), members)(guardrail_name="scripted")  # type: ignore[no-any-return,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L180


def input_result(handler: Handler, scenario: InputScenario) -> JsonValue:
    calls: list[JsonValue] = []
    guardrail = scripted_guardrail(scenario.get("reply"), calls)
    data = dict(scenario["data"])
    asyncio.run(handler.process_input_messages(data, guardrail))
    result: dict[str, JsonValue] = {"data": data, "calls": calls}
    return result


def output_result(handler: Handler, scenario: OutputScenario) -> JsonValue:
    calls: list[JsonValue] = []
    guardrail = scripted_guardrail(scenario.get("reply"), calls)
    response = scenario["response"]
    asyncio.run(
        handler.process_output_response(
            response, guardrail, request_data=scenario.get("request_data")
        )
    )
    result: dict[str, JsonValue] = {"response": response, "calls": calls}
    return result


def compact_line(text: str) -> JsonValue:
    parsed = _JSON.validate_json(text)
    return json.dumps(parsed, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n"


def normalized_item(item: JsonValue, before: JsonValue) -> JsonValue:
    match item:
        case str() if item != before:
            return compact_line(item)
        case str() | dict() | list() | None | bool() | int() | float():
            return item


def stream_result(handler: Handler, scenario: StreamScenario) -> JsonValue:
    calls: list[JsonValue] = []
    guardrail = scripted_guardrail(scenario.get("reply"), calls)
    responses = _HANDLER_INPUT.validate_python(scenario["responses"])
    before = _RESPONSES.validate_python(responses)
    asyncio.run(
        handler.process_output_streaming_response(
            responses, guardrail, request_data=scenario.get("request_data")
        )
    )
    after = _RESPONSES.validate_python(responses)
    items: list[JsonValue] = [
        normalized_item(item, before[index]) for index, item in enumerate(after)
    ]
    result: dict[str, JsonValue] = {"responses": items, "calls": calls}
    return result


def run(input_value: Input) -> JsonValue:
    handler = A2AGuardrailHandler()
    if input_value["function"] == "input":
        return [input_result(handler, scenario) for scenario in input_value["scenarios"]]
    if input_value["function"] == "output":
        return [output_result(handler, scenario) for scenario in input_value["scenarios"]]
    return [stream_result(handler, scenario) for scenario in input_value["scenarios"]]
