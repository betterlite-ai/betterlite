from typing import Literal, TypeAlias, TypedDict, cast

from litellm.llms.base_llm.guardrail_translation.base_translation import (
    BaseTranslation,
    StreamingScanKey,
    StreamTransformSink,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types import guardrails
from litellm.types.guardrails import (
    BedrockChecksConfigModel,
    BedrockGuardrailConfigModel,
    JavelinGuardrailConfigModel,
    LakeraV2GuardrailConfigModel,
    NomaGuardrailConfigModel,
    PillarGuardrailConfigModel,
    ZscalerAIGuardConfigModel,
)
from pydantic import BaseModel, ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

class ScanKeyInput(TypedDict):
    texts: list[str]
    tool_calls: list[str]
    stream_ended: bool
    tool_calls_in_flight: bool


class Input(TypedDict):
    function: Literal[
        "bedrock_checks",
        "bedrock_guardrail",
        "bedrock_streaming_from_extras",
        "provider_config",
        "scan_key",
        "user_metadata",
        "merge_metadata",
    ]
    model: Literal["bedrock", "lakera_v2", "pillar", "noma", "zscaler", "javelin"]
    body: dict[str, Json]
    extras: dict[str, Json] | None
    request_data: dict[str, Json]
    user_api_key_dict: dict[str, Json] | None
    key: ScanKeyInput
    other_key: ScanKeyInput
    sink_mutated: dict[str, str]
    sink_holdback: dict[str, int]


def config_answer(function: str, body: dict[str, Json]) -> Json | None:
    match function:
        case "bedrock_checks":
            return validate(BedrockChecksConfigModel, body)
        case "bedrock_guardrail":
            return validate(BedrockGuardrailConfigModel, body)
        case _:
            return None


def provider_model(name: str) -> type[BaseModel]:
    match name:
        case "bedrock":
            return BedrockGuardrailConfigModel
        case "lakera_v2":
            return cast("type[BaseModel]", LakeraV2GuardrailConfigModel)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L709-L732
        case "pillar":
            return PillarGuardrailConfigModel
        case "noma":
            return NomaGuardrailConfigModel
        case "zscaler":
            return ZscalerAIGuardConfigModel
        case "javelin":
            return cast("type[BaseModel]", JavelinGuardrailConfigModel)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L820-L833
        case _:
            raise ValueError(f"unknown guardrail model {name}")


def validate(model: type[BaseModel], body: dict[str, Json]) -> Json:
    try:
        validated = model.model_validate(body)
    except ValidationError:
        return {"accepted": False}
    dump = cast(Json, validated.model_dump(mode="json"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L594-L833
    return {"accepted": True, "dump": dump}


def streaming_from_extras(extras: dict[str, Json] | None) -> Json:
    try:
        params = guardrails.BedrockGuardrailStreamingParams.from_extras(extras)
    except ValidationError:
        return {"accepted": False}
    dump = cast(Json, params.model_dump(mode="json"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L669-L707
    return {"accepted": True, "dump": dump}


def to_scan_key(fields: ScanKeyInput) -> StreamingScanKey:
    return StreamingScanKey(
        texts=tuple(fields["texts"]),
        tool_calls=tuple(fields["tool_calls"]),
        stream_ended=fields["stream_ended"],
        tool_calls_in_flight=fields["tool_calls_in_flight"],
    )


def scan_key(input_value: Input) -> Json:
    sink = StreamTransformSink()
    for index, text in input_value["sink_mutated"].items():
        sink.mutated_text_per_choice[int(index)] = text
    for index, holdback in input_value["sink_holdback"].items():
        sink.holdback_per_choice[int(index)] = holdback
    key = to_scan_key(input_value["key"])
    other = to_scan_key(input_value["other_key"])
    mutated: dict[str, Json] = {
        str(index): text for index, text in sink.mutated_text_per_choice.items()
    }
    holdbacks: dict[str, Json] = {
        str(index): value for index, value in sink.holdback_per_choice.items()
    }
    return {
        "equal": key == other,
        "has_nothing_to_scan": key.has_nothing_to_scan,
        "mutated": mutated,
        "holdback": holdbacks,
    }


def user_metadata(input_value: Input) -> Json:
    metadata = BaseTranslation.transform_user_api_key_dict_to_metadata(
        input_value["user_api_key_dict"]
    )
    return cast(Json, metadata)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/guardrail_translation/base_translation.py#L83-L120


def merge_metadata(input_value: Input) -> Json:
    request_data = dict(input_value["request_data"])
    BaseTranslation.merge_user_api_key_metadata_into_request(
        request_data,
        cast("UserAPIKeyAuth | None", input_value["user_api_key_dict"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/guardrail_translation/base_translation.py#L123-L155
    )
    return request_data


def run(input_value: Input) -> Json:
    function = input_value["function"]
    config = config_answer(function, input_value["body"])
    if config is not None:
        return config
    match function:
        case "bedrock_streaming_from_extras":
            return streaming_from_extras(input_value["extras"])
        case "provider_config":
            return validate(provider_model(input_value["model"]), input_value["body"])
        case "scan_key":
            return scan_key(input_value)
        case "user_metadata":
            return user_metadata(input_value)
        case _:
            return merge_metadata(input_value)
