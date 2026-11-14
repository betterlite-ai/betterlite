from collections.abc import Mapping
from typing import ClassVar, Final, Literal, cast

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.utils import ProxyLogging
from litellm.types.utils import LLMResponseTypes
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ScanInputs(TypedDict, total=False):
    texts: list[str]
    model: str | None


class Answer(TypedDict):
    texts: list[str]


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


class ScriptedGuardrail(
    cast("type[CustomGuardrail]", CustomGuardrail)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L209
):
    # ast-grep-ignore: no-long-names
    records_own_guardrail_information: ClassVar[bool] = True
    # ast-grep-ignore: no-long-names
    skip_system_message_in_guardrail: bool | None = None
    skip_tool_message_in_guardrail: bool | None = None

    def __init__(
        self,
        answer: list[str] | None,
        *,
        skip_system: bool,
        skip_tool: bool,
    ) -> None:
        self.answer = answer
        self.texts: list[str] = []
        self.model: str | None = None
        self.skip_system_message_in_guardrail = skip_system
        self.skip_tool_message_in_guardrail = skip_tool

    @override
    async def apply_guardrail(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_guardrail.py#L1312-L1318
        self,
        inputs: ScanInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: Logging | None = None,
    ) -> Answer:
        texts = inputs.get("texts")
        self.texts = [] if texts is None else list(texts)
        self.model = inputs.get("model")
        if self.answer is None:
            return {"texts": self.texts}
        return {"texts": self.answer}


class ScriptedHook(ProxyLogging):
    replace: dict[str, str]

    def __init__(self, replace: dict[str, str]) -> None:
        self.replace = replace

    @override
    async def post_call_success_hook(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3494-L3499
        self,
        data: Mapping[str, JsonValue],
        response: LLMResponseTypes,
        user_api_key_dict: UserAPIKeyAuth,
    ) -> JsonValue:
        synthetic = SYNTHETIC.validate_python(response)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3494-L3499
        for block in synthetic["output"]["message"]["content"]:
            replacement = self.replace.get(block["text"])
            if replacement is not None:
                block["text"] = replacement
        return JSON.validate_python(synthetic)
