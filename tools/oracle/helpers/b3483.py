from datetime import UTC, datetime
from typing import TypedDict

from litellm.google_genai import main as g
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.google_genai.transformation import (
    BaseGoogleGenAIGenerateContentConfig as BaseConfig,
)
from litellm.llms.gemini.google_genai.transformation import GoogleGenAIConfig
from litellm.llms.vertex_ai.google_genai.transformation import VertexAIGoogleGenAIConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter


class SetupCall(TypedDict):
    model: str
    contents: JsonValue
    config: dict[str, JsonValue] | None
    custom_llm_provider: str | None
    tools: JsonValue | None
    kwargs: dict[str, JsonValue]
    extra_body: dict[str, JsonValue] | None
    with_logging: bool


class Input(TypedDict):
    setups: list[SetupCall]


class LoggingDetails(BaseModel):
    model: str | None = None
    optional_params: dict[str, JsonValue] | None = None
    litellm_params: dict[str, JsonValue] | None = None
    custom_llm_provider: str | None = None
    litellm_call_id: str | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: LoggingDetails


INPUT: TypeAdapter[Input] = TypeAdapter(Input)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
MERGED: TypeAdapter[dict[str, JsonValue] | None] = TypeAdapter(
    dict[str, JsonValue] | None
)
MODELED_PARAMS: frozenset[str] = frozenset(
    {
        "api_base",
        "api_key",
        "api_version",
        "custom_llm_provider",
        "max_retries",
        "organization",
        "mock_response",
        "vertex_project",
        "vertex_location",
        "vertex_credentials",
        "ssl_verify",
    }
)


MERGE, SETUP = g._merge_native_request_fields, g.GenerateContentHelper.setup_generate_content_call  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/google_genai/main.py#L91-L98


def make_logging() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="generate_content",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def dumped_params(params: GenericLiteLLMParams) -> dict[str, JsonValue]:
    # extra="allow" keeps every kwarg the setup does not model
    return OBJECT.validate_json(
        params.model_dump_json(exclude_unset=True, exclude={"litellm_logging_obj"})
    )


def params_view(params: GenericLiteLLMParams) -> dict[str, JsonValue]:
    dumped = dumped_params(params)
    return {
        "api_base": params.api_base,
        "api_version": params.api_version,
        "custom_llm_provider": params.custom_llm_provider,
        "max_retries": params.max_retries,
        "organization": params.organization,
        "api_key_set": params.api_key is not None,
        "vertex_project": params.vertex_project,
        "vertex_location": params.vertex_location,
        "vertex_credentials": dumped.get("vertex_credentials"),
        "extra": {
            name: value
            for name, value in dumped.items()
            if name not in MODELED_PARAMS
        },
    }


def logging_view(logging_obj: Logging) -> JsonValue:
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    return {
        "model": details.model,
        "optional_params": details.optional_params,
        "litellm_params": details.litellm_params,
        "custom_llm_provider": details.custom_llm_provider,
        "litellm_call_id": details.litellm_call_id,
    }


def setup_answer(call: SetupCall) -> JsonValue:
    logging_obj = make_logging() if call["with_logging"] else None
    result = SETUP(
        model=call["model"],
        contents=call["contents"],
        config=call["config"],
        custom_llm_provider=call["custom_llm_provider"],
        tools=call["tools"],
        litellm_logging_obj=logging_obj,
        **call["kwargs"],
    )
    native = OBJECT.validate_python(result.native_request_fields)
    provider_config = result.generate_content_provider_config
    provider_kind: str | None = None
    if provider_config is not None:
        match provider_config:
            case VertexAIGoogleGenAIConfig() | GoogleGenAIConfig() as known:
                provider_kind = known.custom_llm_provider
            case BaseConfig():
                provider_kind = None
    return {
        "model": result.model,
        "provider": result.custom_llm_provider,
        "provider_config": provider_kind,
        "config_dict": OBJECT.validate_python(result.generate_content_config_dict),
        "native_request_fields": native,
        "request_body": OBJECT.validate_python(result.request_body),
        "litellm_params": params_view(result.litellm_params),
        "merged_extra_body": MERGED.validate_python(
            MERGE(
                result.native_request_fields,
                None if call["extra_body"] is None else {**call["extra_body"]},
            )
        ),
        "logging": logging_view(result.litellm_logging_obj),
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"setups": [setup_answer(call) for call in parsed["setups"]]}
