import uuid
from dataclasses import dataclass
from typing import Annotated, Final, NotRequired, TypedDict, cast

import httpx
import litellm
from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import AliasPath, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON_VALUE: Final = TypeAdapter(JsonValue)
CHAT_COMPLETION_PREFIX: Final = "chatcmpl-"


class Input(TypedDict):
    model: str
    legacy_function: bool
    now: int
    generated_id: str
    headers: dict[str, str]
    body: dict[str, JsonValue]


class ProviderFields(TypedDict):
    traffic_type: NotRequired[str]


class HiddenParams(TypedDict, total=False):
    provider_specific_fields: ProviderFields


@dataclass
class Logging:
    optional_params: dict[str, JsonValue]

    def post_call(self, **_kwargs: JsonValue) -> None: ...


@dataclass
class ChosenMessage:
    role: str
    content: JsonValue = None
    reasoning_content: JsonValue = None
    thinking_blocks: JsonValue = None
    tool_calls: JsonValue = None
    function_call: JsonValue = None
    images: JsonValue = None
    audio: JsonValue = None
    annotations: JsonValue = None
    provider_specific_fields: JsonValue = None


@dataclass
class Chosen:
    index: int
    finish_reason: str
    message: ChosenMessage
    logprobs: JsonValue = None
    native_finish_reason: Annotated[
        JsonValue,
        Field(validation_alias=AliasPath("provider_specific_fields", "native_finish_reason")),
    ] = None


@dataclass
class UsageState:
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: JsonValue = None
    completion_tokens_details: JsonValue = None


@dataclass
class Envelope:
    id: str
    model: str
    choices: list[Chosen]
    usage: UsageState
    service_tier: JsonValue = None
    grounding_metadata: Annotated[
        JsonValue, Field(validation_alias="vertex_ai_grounding_metadata")
    ] = None
    url_context_metadata: Annotated[
        JsonValue, Field(validation_alias="vertex_ai_url_context_metadata")
    ] = None
    safety_results: Annotated[JsonValue, Field(validation_alias="vertex_ai_safety_results")] = None
    citation_metadata: Annotated[
        JsonValue, Field(validation_alias="vertex_ai_citation_metadata")
    ] = None


ENVELOPE: Final = TypeAdapter(Envelope)
CHOICES: Final = TypeAdapter(list[Chosen])
HIDDEN: Final = TypeAdapter(HiddenParams)


def _transform(input_value: Input) -> ModelResponse:
    optional_params: dict[str, JsonValue] = (
        {"functions": [{}]} if input_value["legacy_function"] else {}
    )
    generated = f"{CHAT_COMPLETION_PREFIX}{uuid.UUID(hex=input_value['generated_id'])}"
    request_data: dict[str, JsonValue] = {}
    litellm_params: dict[str, JsonValue] = {}
    return VertexGeminiConfig().transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L2372
        model=input_value["model"],
        raw_response=httpx.Response(
            200,
            json=input_value["body"],
            headers=input_value["headers"],
        ),
        model_response=ModelResponse(id=generated),
        logging_obj=cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L109
            "gemini.LoggingClass", Logging(optional_params=optional_params)
        ),
        request_data=request_data,
        messages=cast("list[AllMessageValues]", []),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1
        optional_params=optional_params,
        litellm_params=litellm_params,
        encoding=None,
    )


def _traffic_type(response: ModelResponse) -> JsonValue:
    hidden = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
        dict[str, JsonValue],
        response._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    )
    fields = HIDDEN.validate_python(hidden)
    provider = fields.get("provider_specific_fields")
    if provider is None:
        return None
    return provider.get("traffic_type")


def _metadata(envelope: Envelope) -> JsonValue:
    if envelope.grounding_metadata is None:
        return None
    return {
        "grounding_metadata": envelope.grounding_metadata,
        "url_context_metadata": envelope.url_context_metadata,
        "safety_results": envelope.safety_results,
        "citation_metadata": envelope.citation_metadata,
    }


def _usage(usage: UsageState) -> JsonValue:
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": usage.prompt_tokens_details is not None,
        "completion_tokens_details": usage.completion_tokens_details is not None,
    }


def _state(response: ModelResponse) -> JsonValue:
    envelope = ENVELOPE.validate_json(response.model_dump_json())
    choices = JSON_VALUE.validate_json(CHOICES.dump_json(envelope.choices))
    return {
        "id": envelope.id,
        "model": envelope.model,
        "choices": choices,
        "usage": _usage(envelope.usage),
        "metadata": _metadata(envelope),
        "traffic_type": _traffic_type(response),
        "service_tier": envelope.service_tier,
    }


def run(input_value: Input) -> JsonValue:
    response = _transform(input_value)
    return _state(response)
