import logging
from logging.handlers import MemoryHandler
from typing import Literal, TypedDict, cast

import litellm
from litellm.vector_store_files import main as vector_store_files_main
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ScriptedError(TypedDict):
    class_name: Literal["ValueError", "RuntimeError"]
    message: str


class RegistryScript(TypedDict):
    result: dict[str, JsonValue] | None
    error: ScriptedError | None


class Input(TypedDict):
    vector_store_id: str
    custom_llm_provider: str | None
    kwargs: dict[str, JsonValue]
    registry: RegistryScript | None


class ScriptedRegistry(VectorStoreRegistry):
    def __init__(self, script: RegistryScript, calls: list[str]) -> None:
        super().__init__()
        self._script = script
        self._calls = calls

    @override
    def get_credentials_for_vector_store(  # ast-grep-ignore: no-long-names
        self, vector_store_id: str
    ) -> dict[str, object]:  # ast-grep-ignore: no-loose-types
        self._calls.append(vector_store_id)
        error = self._script["error"]
        if error is not None and error["class_name"] == "ValueError":
            raise ValueError(error["message"])
        if error is not None:
            raise RuntimeError(error["message"])
        result = self._script["result"]
        if result is None:
            return {}
        return cast("dict[str, object]", result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L513-L526


def run(input_value: Input) -> JsonValue:
    provider = vector_store_files_main._ensure_provider(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L35-L36
        input_value["custom_llm_provider"]
    )
    kwargs = cast("dict[str, object]", input_value["kwargs"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L39-L51
    calls: list[str] = []
    registry = input_value["registry"]
    previous = litellm.vector_store_registry
    litellm.vector_store_registry = None if registry is None else ScriptedRegistry(registry, calls)
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        vector_store_files_main._prepare_registry_credentials(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L39-L51
            vector_store_id=input_value["vector_store_id"], kwargs=kwargs
        )
    finally:
        logger.removeHandler(capture)
        litellm.vector_store_registry = previous
    warnings = [record.getMessage() for record in capture.buffer]
    final_kwargs = cast("dict[str, JsonValue]", kwargs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L39-L51
    projection: dict[str, JsonValue] = {
        "provider": provider,
        "kwargs": final_kwargs,
        "lookups": JSON_VALUE.validate_python(calls),
        "warnings": JSON_VALUE.validate_python(warnings),
    }
    return projection
