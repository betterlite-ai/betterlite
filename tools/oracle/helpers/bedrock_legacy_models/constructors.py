from typing import Literal, Protocol, TypedDict

import litellm
from litellm.llms.bedrock.chat.invoke_transformations.amazon_ai21_transformation import (
    AmazonAI21Config,
)
from litellm.llms.bedrock.chat.invoke_transformations.amazon_cohere_transformation import (
    AmazonCohereConfig,
)
from litellm.llms.bedrock.chat.invoke_transformations.amazon_llama_transformation import (
    AmazonLlamaConfig,
)
from litellm.llms.bedrock.chat.invoke_transformations.amazon_mistral_transformation import (
    AmazonMistralConfig,
)
from litellm.llms.bedrock.chat.invoke_transformations.amazon_titan_transformation import (
    AmazonTitanConfig,
)
from litellm.llms.bedrock.chat.invoke_transformations.anthropic_claude2_transformation import (
    AmazonAnthropicConfig,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

FamilyName = Literal["ai21", "cohere", "llama", "mistral", "titan", "anthropic"]

# The bare list and dict parameters make these class objects read as Any; the names below carry the class type.
AI21: type[AmazonAI21Config] = AmazonAI21Config
COHERE: type[AmazonCohereConfig] = AmazonCohereConfig
LLAMA: type[AmazonLlamaConfig] = AmazonLlamaConfig
MISTRAL: type[AmazonMistralConfig] = AmazonMistralConfig
TITAN: type[AmazonTitanConfig] = AmazonTitanConfig
ANTHROPIC: type[AmazonAnthropicConfig] = AmazonAnthropicConfig


class LegacyClass(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, Json]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...


class Ai21Args(TypedDict, total=False):
    maxTokens: int | None
    temperature: int | float | None
    topP: int | float | None
    stopSequences: list[str] | None
    frequencePenalty: dict[str, Json] | None
    presencePenalty: dict[str, Json] | None
    countPenalty: dict[str, Json] | None


class CohereArgs(TypedDict, total=False):
    max_tokens: int | None
    temperature: int | float | None
    return_likelihood: str | None


class LlamaArgs(TypedDict, total=False):
    maxTokenCount: int | None
    temperature: int | float | None
    topP: int | None


class MistralArgs(TypedDict, total=False):
    max_tokens: int | None
    temperature: int | float | None
    top_p: int | None
    top_k: int | float | None
    stop: list[str] | None


class TitanArgs(TypedDict, total=False):
    maxTokenCount: int | None
    stopSequences: list[str] | None
    temperature: int | float | None
    topP: int | None


class AnthropicArgs(TypedDict, total=False):
    max_tokens_to_sample: int | None
    stop_sequences: list[str] | None
    temperature: int | float | None
    top_k: int | None
    top_p: int | None
    anthropic_version: str | None


class ConstructorRow(TypedDict):
    family: FamilyName
    calls: list[dict[str, Json]]


class Constructors(TypedDict):
    default_max_tokens: int
    cases: list[ConstructorRow]


def construct(family: FamilyName, values: dict[str, Json]) -> LegacyClass:
    if family == "ai21":
        return AmazonAI21Config(**TypeAdapter(Ai21Args).validate_python(values))
    if family == "cohere":
        return AmazonCohereConfig(**TypeAdapter(CohereArgs).validate_python(values))
    if family == "llama":
        return AmazonLlamaConfig(**TypeAdapter(LlamaArgs).validate_python(values))
    if family == "mistral":
        return AmazonMistralConfig(**TypeAdapter(MistralArgs).validate_python(values))
    if family == "titan":
        return AmazonTitanConfig(**TypeAdapter(TitanArgs).validate_python(values))
    return AmazonAnthropicConfig(**TypeAdapter(AnthropicArgs).validate_python(values))


def reset(family: FamilyName) -> None:
    if family == "ai21":
        AI21.maxTokens = None
        AI21.temperature = None
        AI21.topP = None
        AI21.stopSequences = None
        AI21.frequencePenalty = None
        AI21.presencePenalty = None
        AI21.countPenalty = None
    elif family == "cohere":
        COHERE.max_tokens = None
        COHERE.temperature = None
        COHERE.return_likelihood = None
    elif family == "llama":
        # maxTokenCount is a constructor keyword the class never declares, so it cannot be cleared
        LLAMA.temperature = None
        LLAMA.topP = None
    elif family == "mistral":
        MISTRAL.max_tokens = None
        MISTRAL.temperature = None
        MISTRAL.top_p = None
        MISTRAL.top_k = None
        MISTRAL.stop = None
    elif family == "titan":
        TITAN.maxTokenCount = None
        TITAN.stopSequences = None
        TITAN.temperature = None
        TITAN.topP = None
    else:
        ANTHROPIC.max_tokens_to_sample = litellm.max_tokens
        ANTHROPIC.stop_sequences = None
        ANTHROPIC.temperature = None
        ANTHROPIC.top_k = None
        ANTHROPIC.top_p = None
        ANTHROPIC.anthropic_version = None


def encoded(name: str, value: Json) -> Json:
    if name == "_abc_impl":
        return {"kind": "abc_data"}
    if name == "descriptor":
        return {"kind": "descriptor"}
    return value


def project(view: LegacyClass) -> Json:
    config = view.get_config()
    return {name: encoded(name, value) for name, value in config.items()}


def construct_row(row: ConstructorRow) -> Json:
    family = row["family"]
    view = construct(family, {})
    reset(family)
    initial = project(view)
    after: list[Json] = [project(construct(family, call)) for call in row["calls"]]
    return {"initial": initial, "after": after}


def constructors(raw: Json) -> Json:
    cases = TypeAdapter(Constructors).validate_python(raw)
    # Claude2's class default is the module's litellm.max_tokens, so the input carries it.
    previous = litellm.max_tokens
    litellm.max_tokens = cases["default_max_tokens"]
    try:
        rows: list[Json] = [construct_row(row) for row in cases["cases"]]
    finally:
        litellm.max_tokens = previous
    return {"results": rows}
