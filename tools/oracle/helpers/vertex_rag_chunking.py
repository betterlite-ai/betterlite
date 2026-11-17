import logging
from logging.handlers import MemoryHandler
from typing import NotRequired, Protocol, TypeAlias, TypedDict

from litellm.llms.vertex_ai.rag_engine.transformation import VertexAIRAGTransformation
from litellm.types.rag import RAGChunkingStrategy
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]


class Case(TypedDict):
    chunking_strategy: NotRequired[JsonObject | None]


class Input(TypedDict):
    cases: list[Case]


class ChunkingMapper(Protocol):
    def transform_chunking_strategy_to_vertex_format(  # ast-grep-ignore: no-long-names
        self, chunking_strategy: RAGChunkingStrategy | None
    ) -> JsonValue: ...


INPUT = TypeAdapter(Input)
STRATEGY = TypeAdapter(RAGChunkingStrategy)


def transform(case: Case) -> JsonValue:
    value = case.get("chunking_strategy")
    strategy = None if value is None else STRATEGY.validate_python(value)
    mapper: ChunkingMapper = VertexAIRAGTransformation()  # type: ignore[no-untyped-call,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/rag_engine/transformation.py#L26-L28
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        chunking = mapper.transform_chunking_strategy_to_vertex_format(strategy)
    finally:
        logger.removeHandler(capture)
    return {
        "config": chunking,
        "warnings": [entry.getMessage() for entry in capture.buffer],
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"cases": [transform(case) for case in parsed["cases"]]}
