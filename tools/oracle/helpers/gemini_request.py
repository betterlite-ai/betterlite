from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.gemini import transformation
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JsonObject: TypeAlias = dict[str, Json]
Provider: TypeAlias = Literal["vertex_ai", "vertex_ai_beta", "gemini"]


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
    custom_llm_provider: Provider
    litellm_params: JsonObject
    vertex_project: None
    vertex_location: None
    vertex_auth_header: None


class Transform(Protocol):
    def __call__(self, **kwargs: Unpack[TransformArgs]) -> Mapping[str, Json]: ...


# LiteLLM's request body takes bare dicts and its RequestBody TypedDicts carry Any
TRANSFORM: Transform = transformation.sync_transform_request_body  # type: ignore[assignment]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1262-L1277
START_TIME = datetime(2026, 1, 1, tzinfo=UTC)
JSON_OBJECT = TypeAdapter(JsonObject)

litellm.suppress_debug_info = True


class Supports(TypedDict):
    system_message: bool
    response_schema: bool


class Case(TypedDict):
    model: str
    provider: Provider
    # the Python side reads LiteLLM's own catalog, so this states what the Rust fixture answers
    supports: Supports
    modify_params: bool
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject


class Input(TypedDict):
    cases: list[Case]


class Result(TypedDict):
    body: JsonObject
    litellm_params: JsonObject


class Output(TypedDict):
    results: list[Result]


def case_result(case: Case) -> Result:
    litellm.modify_params = case["modify_params"]
    logging_object = Logging(
        model=case["model"],
        messages=None,
        stream=False,
        call_type="completion",
        start_time=START_TIME,
        litellm_call_id="gemini-request",
        function_id="gemini-request",
        supports_correlation_logging=False,
    )
    optional_params = dict(case["optional_params"])
    litellm_params = dict(case["litellm_params"])
    request = TRANSFORM(
        gemini_api_key=None,
        messages=case["messages"],
        api_base=None,
        model=case["model"],
        client=None,
        timeout=None,
        extra_headers=None,
        optional_params=optional_params,
        logging_obj=logging_object,
        custom_llm_provider=case["provider"],
        litellm_params=litellm_params,
        vertex_project=None,
        vertex_location=None,
        vertex_auth_header=None,
    )
    return {
        "body": JSON_OBJECT.validate_python(dict(request)),
        "litellm_params": JSON_OBJECT.validate_python(dict(litellm_params)),
    }


def run(input_value: Input) -> Output:
    return {"results": [case_result(case) for case in input_value["cases"]]}
