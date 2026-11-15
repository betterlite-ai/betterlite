from typing import TypedDict

from litellm.llms.gemini.common_utils import GeminiModelInfo
from litellm.llms.vertex_ai.common_utils import VertexAIModelInfo


class CounterRow(TypedDict):
    kind: str
    selected: list[bool]


class Input(TypedDict):
    providers: list[str | None]


class Output(TypedDict):
    vertex: CounterRow
    studio: CounterRow


def run(input_value: Input) -> Output:
    vertex = VertexAIModelInfo().get_token_counter()
    studio = GeminiModelInfo().get_token_counter()
    if vertex is None or studio is None:
        raise ValueError("a Google model-info factory returned no token counter")
    providers = input_value["providers"]
    return {
        "vertex": {
            "kind": type(vertex).__name__,
            "selected": [vertex.should_use_token_counting_api(provider) for provider in providers],
        },
        "studio": {
            "kind": type(studio).__name__,
            "selected": [studio.should_use_token_counting_api(provider) for provider in providers],
        },
    }
