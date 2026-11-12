import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.llms.anthropic import compaction
from litellm.llms.compaction import CompactionProtocol, dispatch, get_native_compaction_provider
from litellm.router import Router
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)
_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
_OBJECTS: Final = TypeAdapter(list[JsonValue])


class SupportsInput(TypedDict):
    function: Literal["supports"]
    model: str
    custom_llm_provider: str | None


class DefaultsInput(TypedDict):
    function: Literal["defaults"]
    payload: dict[str, JsonValue]


class RequestKwargsInput(TypedDict):
    function: Literal["request_kwargs"]


class BlocksInput(TypedDict):
    function: Literal["blocks"]
    protocol: CompactionProtocol
    response: dict[str, JsonValue]


class SummaryInput(TypedDict):
    function: Literal["summary"]
    protocol: CompactionProtocol
    response: dict[str, JsonValue]


class DispatchInput(TypedDict):
    function: Literal["dispatch"]
    protocol: CompactionProtocol
    payload: dict[str, JsonValue]
    reply_id: str


Input: TypeAlias = (
    SupportsInput | DefaultsInput | RequestKwargsInput | BlocksInput | SummaryInput | DispatchInput
)


class RecordingRouter(Router):
    def __init__(self, reply_id: str) -> None:
        self.reply_id = reply_id
        self.calls: list[JsonValue] = []

    @override
    async def aanthropic_messages(self, **kwargs: JsonValue) -> dict[str, str]:
        options = {
            name: value
            for name, value in kwargs.items()
            if name not in ("custom_llm_provider", "client")
        }
        self.calls.append({"route": "messages", "options": options})
        return {"id": self.reply_id}

    @override
    async def acompletion(  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L2779-L2784
        self,
        model: str,
        messages: list[AllMessageValues],
        stream: bool = False,
        **kwargs: JsonValue,
    ) -> ModelResponse:
        self.calls.append(
            {
                "route": "chat",
                "model": model,
                "messages": _JSON.validate_python(messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
                "stream": stream,
                "options": kwargs,
            }
        )
        return ModelResponse(id=self.reply_id)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "supports":
        params = {
            "model": input_value["model"],
            "custom_llm_provider": input_value["custom_llm_provider"],
        }
        return get_native_compaction_provider(params) is not None
    if input_value["function"] == "defaults":
        return compaction.compatible_defaults(input_value["payload"])
    if input_value["function"] == "request_kwargs":
        return _OBJECT.validate_python(compaction.request_kwargs())
    if input_value["function"] == "blocks":
        blocks = compaction._native_blocks(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/compaction.py#L47-L56
            input_value["protocol"], input_value["response"]
        )
        return _OBJECTS.validate_python(blocks)
    if input_value["function"] == "summary":
        return compaction.extract_summary(input_value["protocol"], input_value["response"])
    router = RecordingRouter(input_value["reply_id"])
    result = asyncio.run(dispatch(router, input_value["protocol"], input_value["payload"]))
    return {"calls": router.calls, "result_id": _JSON.validate_python(result["id"])}
