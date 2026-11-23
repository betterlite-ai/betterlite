from collections.abc import Mapping
from typing import Final, Protocol, TypedDict, cast

from litellm.responses.main import _has_file_search_tool, mock_responses_api_response  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L98-L116
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class MockResponse(Protocol):
    def model_dump_json(self) -> str: ...


class MockBuilder(Protocol):
    def __call__(self, *args: str) -> MockResponse: ...


MOCK_BUILDER: Final[MockBuilder] = cast(MockBuilder, mock_responses_api_response)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L105-L116


class LiteralStep(TypedDict):
    text: str | None
    tools: list[list[JsonValue] | None]


class LiteralInput(TypedDict):
    steps: list[LiteralStep]


def run(input_value: LiteralInput) -> JsonValue:
    steps: list[JsonValue] = []
    for step in input_value["steps"]:
        text = step["text"]
        built = MOCK_BUILDER() if text is None else MOCK_BUILDER(text)
        searches: list[JsonValue] = [
            _has_file_search_tool(cast("list[Mapping[str, object]] | None", tools))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L98-L102
            for tools in step["tools"]
        ]
        steps.append({"mock": JSON.validate_json(built.model_dump_json()), "file_search": searches})
    return {"steps": steps}
