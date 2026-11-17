import logging
from collections.abc import Generator
from contextlib import contextmanager
from typing import Literal, NotRequired, Protocol, TypedDict

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
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import ModelResponse
from pydantic import JsonValue as Json, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

FamilyName = Literal["ai21", "cohere", "llama", "mistral", "titan", "anthropic"]


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def first(self) -> str | None:
        return self.messages[0] if self.messages else None


# The constructor arguments of each class, named and typed as the pinned __init__ declares them.
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


class MapCase(TypedDict):
    family: FamilyName
    non_default: dict[str, Json]
    optional: dict[str, Json]
    model: str
    drop_params: bool
    global_drop_params: bool


class Mappings(TypedDict):
    cases: list[MapCase]


class ConstructorRow(TypedDict):
    family: FamilyName
    calls: list[dict[str, Json]]


class Constructors(TypedDict):
    default_max_tokens: int
    cases: list[ConstructorRow]


class ProjectionCase(TypedDict):
    family: FamilyName
    attributes: dict[str, Json]


class OutputCase(TypedDict):
    response: dict[str, Json]


class Steps(TypedDict):
    mappings: NotRequired[Mappings]
    constructors: NotRequired[Constructors]
    projection: NotRequired[ProjectionCase]
    output_text: NotRequired[OutputCase]


# A fresh class dict carries _abc_impl; Claude2's limit is the import-time litellm.max_tokens.
class Ai21Kinds(AmazonAI21Config):
    def _method(self) -> None: ...

    __marked__ = "dunder"
    plain = 5
    text = "value"
    nothing = None
    _private = "kept"
    _abc_value = "abc"
    function = _method
    builtin = len
    class_method = classmethod(_method)
    static_method = staticmethod(_method)
    descriptor = property()


class AnthropicKinds(AmazonAnthropicConfig):
    def _method(self) -> None: ...

    __marked__ = "dunder"
    plain = 5
    text = "value"
    nothing = None
    _private = "kept"
    _abc_value = "abc"
    function = _method
    builtin = len
    class_method = classmethod(_method)
    static_method = staticmethod(_method)
    descriptor = property()


@contextmanager
def capturing(*, global_drop_params: bool) -> Generator[Warnings]:
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous = litellm.drop_params
    litellm.drop_params = global_drop_params
    try:
        yield warnings
    finally:
        litellm.drop_params = previous
        logger.removeHandler(warnings)


def reset(family: FamilyName) -> None:
    if family == "ai21":
        AmazonAI21Config.maxTokens = None
        AmazonAI21Config.temperature = None
        AmazonAI21Config.topP = None
        AmazonAI21Config.stopSequences = None
        AmazonAI21Config.frequencePenalty = None
        AmazonAI21Config.presencePenalty = None
        AmazonAI21Config.countPenalty = None
    elif family == "cohere":
        AmazonCohereConfig.max_tokens = None
        AmazonCohereConfig.temperature = None
        AmazonCohereConfig.return_likelihood = None
    elif family == "llama":
        # maxTokenCount is a constructor keyword the class never declares, so it cannot be cleared
        AmazonLlamaConfig.temperature = None
        AmazonLlamaConfig.topP = None
    elif family == "mistral":
        AmazonMistralConfig.max_tokens = None
        AmazonMistralConfig.temperature = None
        AmazonMistralConfig.top_p = None
        AmazonMistralConfig.top_k = None
        AmazonMistralConfig.stop = None
    elif family == "titan":
        AmazonTitanConfig.maxTokenCount = None
        AmazonTitanConfig.stopSequences = None
        AmazonTitanConfig.temperature = None
        AmazonTitanConfig.topP = None
    else:
        AmazonAnthropicConfig.max_tokens_to_sample = litellm.max_tokens
        AmazonAnthropicConfig.stop_sequences = None
        AmazonAnthropicConfig.temperature = None
        AmazonAnthropicConfig.top_k = None
        AmazonAnthropicConfig.top_p = None
        AmazonAnthropicConfig.anthropic_version = None


def construct(family: FamilyName, values: dict[str, Json]) -> LegacyClass:
    built: LegacyClass
    if family == "ai21":
        built = AmazonAI21Config(**TypeAdapter(Ai21Args).validate_python(values))
    elif family == "cohere":
        built = AmazonCohereConfig(**TypeAdapter(CohereArgs).validate_python(values))
    elif family == "llama":
        built = AmazonLlamaConfig(**TypeAdapter(LlamaArgs).validate_python(values))
    elif family == "mistral":
        built = AmazonMistralConfig(**TypeAdapter(MistralArgs).validate_python(values))
    elif family == "titan":
        built = AmazonTitanConfig(**TypeAdapter(TitanArgs).validate_python(values))
    else:
        built = AmazonAnthropicConfig(**TypeAdapter(AnthropicArgs).validate_python(values))
    return built


def encoded(name: str, value: Json) -> Json:
    if name == "_abc_impl":
        return {"kind": "abc_data"}
    if name == "descriptor":
        return {"kind": "descriptor"}
    return value


def project(view: LegacyClass) -> Json:
    config = view.get_config()
    return {name: encoded(name, value) for name, value in config.items()}


def mapping(case: MapCase) -> Json:
    with capturing(global_drop_params=case["global_drop_params"]) as warnings:
        mapper = construct(case["family"], {})
        mapped = mapper.map_openai_params(
            non_default_params=case["non_default"],
            optional_params=dict(case["optional"]),
            model=case["model"],
            drop_params=case["drop_params"],
        )
        supported: list[Json] = [*mapper.get_supported_openai_params(case["model"])]
    return {"supported": supported, "mapped": mapped, "warning": warnings.first()}


def mappings(cases: Mappings) -> Json:
    results: list[Json] = [mapping(case) for case in cases["cases"]]
    return {"results": results}


def construct_row(row: ConstructorRow) -> Json:
    family = row["family"]
    reset(family)
    initial = project(construct(family, {}))
    after: list[Json] = [project(construct(family, call)) for call in row["calls"]]
    return {"initial": initial, "after": after}


def constructors(cases: Constructors) -> Json:
    # Claude2's class default is the module's litellm.max_tokens, so the input carries it.
    previous = litellm.max_tokens
    litellm.max_tokens = cases["default_max_tokens"]
    try:
        rows: list[Json] = [construct_row(row) for row in cases["cases"]]
    finally:
        litellm.max_tokens = previous
    return {"results": rows}


def projection(case: ProjectionCase) -> Json:
    if case["family"] == "anthropic":
        view: LegacyClass = AnthropicKinds()
    elif case["family"] == "ai21":
        view = Ai21Kinds()
    else:
        raise ValueError(f"no fixture class for {case['family']}")
    return {"family": case["family"], "config": project(view)}


def output(case: OutputCase) -> Json:
    model_response = ModelResponse()
    try:
        text = AmazonMistralConfig.get_outputText(case["response"], model_response)
    except BedrockError as error:
        return {"error": {"status": error.status_code, "message": error.message}}
    return {"text": text, "finish_reason": model_response.choices[0].finish_reason}


def run(argument: Steps) -> Json:
    result: Json
    if "mappings" in argument:
        result = mappings(TypeAdapter(Mappings).validate_python(argument["mappings"]))
    elif "constructors" in argument:
        result = constructors(TypeAdapter(Constructors).validate_python(argument["constructors"]))
    elif "projection" in argument:
        result = projection(TypeAdapter(ProjectionCase).validate_python(argument["projection"]))
    elif "output_text" in argument:
        result = output(TypeAdapter(OutputCase).validate_python(argument["output_text"]))
    else:
        raise ValueError(f"unsupported step {sorted(argument)}")
    return result
