from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Literal, Protocol, TypedDict, Unpack, cast

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.gemini import transformation
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JsonObject = dict[str, Json]
Provider = Literal["vertex_ai", "vertex_ai_beta", "gemini"]


class TransformArgs(TypedDict):
    gemini_api_key: None
    messages: list[JsonObject]
    api_base: None
    model: str
    client: None
    timeout: None
    extra_headers: None
    optional_params: JsonObject
    logging_obj: Logging
    custom_llm_provider: str
    litellm_params: JsonObject
    vertex_project: None
    vertex_location: None
    vertex_auth_header: None


class Transform(Protocol):
    def __call__(self, **kwargs: Unpack[TransformArgs]) -> Mapping[str, Json]: ...


# LiteLLM's request body takes bare dicts and its RequestBody TypedDicts carry Any
TRANSFORM = cast(Transform, transformation.sync_transform_request_body)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1262-L1277
START_TIME = datetime(2026, 1, 1, tzinfo=UTC)
JSON_OBJECT = TypeAdapter(dict[str, Json])

litellm.suppress_debug_info = True


class Case(TypedDict):
    model: str
    provider: Provider
    # the case also carries a supports object, which only the Rust fixture's capability lookup reads
    modify_params: bool
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject


def run(input_value: Case) -> JsonObject:
    litellm.modify_params = input_value["modify_params"]
    logging_object = Logging(
        model=input_value["model"],
        messages=None,
        stream=False,
        call_type="completion",
        start_time=START_TIME,
        litellm_call_id="gemini-request",
        function_id="gemini-request",
        supports_correlation_logging=False,
    )
    optional_params = dict(input_value["optional_params"])
    litellm_params = dict(input_value["litellm_params"])
    request = TRANSFORM(
        gemini_api_key=None,
        messages=input_value["messages"],
        api_base=None,
        model=input_value["model"],
        client=None,
        timeout=None,
        extra_headers=None,
        optional_params=optional_params,
        logging_obj=logging_object,
        custom_llm_provider=input_value["provider"],
        litellm_params=litellm_params,
        vertex_project=None,
        vertex_location=None,
        vertex_auth_header=None,
    )
    return {
        "body": JSON_OBJECT.validate_python(request),
        "litellm_params": JSON_OBJECT.validate_python(litellm_params),
    }
