from typing import Protocol

from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


# get_config has no return annotation upstream, so the protocol gives the probes its signature
class ConfigProbe(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, JsonValue]: ...


class Mapper(Protocol):
    def map_openai_params(
        self, non_default_params: dict[str, JsonValue], optional_params: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...


class Supported(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...


# an embedding transform both declares its supported OpenAI params and maps them
class EmbeddingConfig(Mapper, Supported, Protocol):
    ...
