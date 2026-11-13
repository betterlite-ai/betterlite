from collections.abc import Mapping
from typing import ClassVar, Final, Literal

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.utils import ProxyLogging
from litellm.types.utils import GenericGuardrailAPIInputs, LLMResponseTypes
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ContentBlock(TypedDict):
    text: str


class Message(TypedDict):
    role: str
    content: list[ContentBlock]


class Output(TypedDict):
    message: Message


class Synthetic(TypedDict):
    output: Output
    stopReason: str


SYNTHETIC: Final[TypeAdapter[Synthetic]] = TypeAdapter(Synthetic)


class ScriptedGuardrail(CustomGuardrail):
    records_own_guardrail_information: ClassVar[bool] = True
    skip_system_message_in_guardrail: bool | None = None
    skip_tool_message_in_guardrail: bool | None = None

    def __init__(self, answer: list[str] | None, skip_system: bool, skip_tool: bool) -> None:
        super().__init__(guardrail_name="scripted")
        self.answer = answer
        self.texts: list[str] = []
        self.model: str | None = None
        self.skip_system_message_in_guardrail = skip_system
        self.skip_tool_message_in_guardrail = skip_tool

    @override
    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> GenericGuardrailAPIInputs:
        texts = inputs.get("texts")
        self.texts = [] if texts is None else list(texts)
        self.model = inputs.get("model")
        if self.answer is None:
            return inputs
        return {"texts": self.answer}


class ScriptedHook(ProxyLogging):
    replace: dict[str, str]

    def __init__(self, replace: dict[str, str]) -> None:
        self.replace = replace

    @override
    async def post_call_success_hook(
        self,
        data: Mapping[str, JsonValue],
        response: JsonValue | LLMResponseTypes,
        user_api_key_dict: UserAPIKeyAuth,
    ) -> JsonValue | LLMResponseTypes:
        synthetic = SYNTHETIC.validate_python(response)
        for block in synthetic["output"]["message"]["content"]:
            block["text"] = self.replace.get(block["text"], block["text"])
        return JSON.validate_python(synthetic)
