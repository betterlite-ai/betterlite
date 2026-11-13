import asyncio
import sys
from types import ModuleType
from typing import NotRequired, TypedDict, cast

from litellm.llms.gemini.count_tokens.handler import GoogleAIStudioTokenCounter
from pydantic import BaseModel, ConfigDict, JsonValue
from pydantic.alias_generators import to_camel


class _FunctionResponse(BaseModel):
    model_config = ConfigDict(alias_generator=to_camel, populate_by_name=True, extra="forbid")

    will_continue: bool | None = None
    scheduling: str | None = None
    parts: list[JsonValue] | None = None
    id: str | None = None
    name: str | None = None
    response: dict[str, JsonValue] | None = None


# google-genai is absent from the oracle venv, so the record the pinned counter projects through is
# carried here: the same fields, aliases, extra-forbid and None-dropping google-genai declares
class _GoogleGenAiTypes(ModuleType):
    FunctionResponse: type[_FunctionResponse] = _FunctionResponse


class _GoogleGenAi(ModuleType):
    types: ModuleType = _GoogleGenAiTypes("google.genai.types")


sys.modules["google.genai"] = _GoogleGenAi("google.genai")
sys.modules["google.genai.types"] = _GoogleGenAiTypes("google.genai.types")


class Call(TypedDict):
    model: str
    contents: JsonValue
    api_base: NotRequired[str]


class Input(TypedDict):
    calls: list[Call]


def run(input_value: Input) -> dict[str, JsonValue]:
    counter = GoogleAIStudioTokenCounter()
    calls: list[JsonValue] = []
    for call in input_value["calls"]:
        _headers, url = asyncio.run(
            counter.validate_environment(
                api_base=call.get("api_base"),
                api_key=None,
                headers={},
                model=call["model"],
                litellm_params={},
            )
        )
        cleaned = cast(JsonValue, counter._clean_contents_for_gemini_api(call["contents"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/count_tokens/handler.py#L16
        calls.append({"contents": cleaned, "original": call["contents"], "url": url})
    return {"calls": calls}
