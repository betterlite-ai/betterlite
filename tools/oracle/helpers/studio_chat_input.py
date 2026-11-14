from datetime import UTC, datetime
from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.chat.transformation import GoogleAIStudioGeminiConfig
from litellm.llms.vertex_ai.gemini.transformation import sync_transform_request_body
from litellm.types.llms.openai import AllMessageValues
from openai import BadRequestError
from pydantic import JsonValue as Json, TypeAdapter

JsonObject: TypeAlias = dict[str, Json]
START_TIME = datetime(2026, 1, 1, tzinfo=UTC)
JSON = TypeAdapter(Json)

litellm.suppress_debug_info = True


class Step(TypedDict):
    op: Literal["params", "messages"]
    model: str
    messages: list[AllMessageValues]


class Input(TypedDict):
    steps: list[Step]
    reasoning_models: NotRequired[list[str]]


def answer(step: Step) -> Json:
    if step["op"] == "params":
        return JSON.validate_python(
            GoogleAIStudioGeminiConfig().get_supported_openai_params(step["model"])
        )
    logging_object = Logging(
        model=step["model"],
        messages=None,
        stream=False,
        call_type="completion",
        start_time=START_TIME,
        litellm_call_id="studio-chat-input",
        function_id="studio-chat-input",
        supports_correlation_logging=False,
    )
    optional_params: JsonObject = {}
    litellm_params: JsonObject = {}
    try:
        request = sync_transform_request_body(
            gemini_api_key=None,
            messages=step["messages"],
            api_base=None,
            model=step["model"],
            client=None,
            timeout=None,
            extra_headers=None,
            optional_params=optional_params,
            logging_obj=logging_object,
            custom_llm_provider="gemini",
            litellm_params=litellm_params,
            vertex_project=None,
            vertex_location=None,
            vertex_auth_header=None,
        )
    except BadRequestError as error:
        return {
            "error": {
                "status": error.status_code,
                "type": error.type,
                "param": error.param,
                "code": str(error.status_code),
            }
        }
    return {"contents": JSON.validate_python(request["contents"])}


def run(input_value: Input) -> Json:
    results: list[Json] = [answer(step) for step in input_value["steps"]]
    return {"results": results}
