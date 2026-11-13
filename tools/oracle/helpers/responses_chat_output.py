from typing import Final, Protocol, TypedDict, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as Config,
)
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

WIRE = TypeAdapter(JsonValue)


class TransformFn(Protocol):
    def __call__(
        self,
        request_input: str,
        responses_api_request: dict[str, JsonValue],
        chat_completion_response: ModelResponse,
    ) -> ResponsesAPIResponse: ...


CONFIG: Final = cast(TransformFn, Config.transform_chat_completion_response_to_responses_api_response)  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L2406


class ChatOutputInput(TypedDict):
    request_input: str
    responses_api_request: dict[str, JsonValue]
    chat_completion_response: dict[str, JsonValue]


def run(input_value: ChatOutputInput) -> JsonValue:
    response = ModelResponse(**input_value["chat_completion_response"])
    result = CONFIG(
        request_input=input_value["request_input"],
        responses_api_request=input_value["responses_api_request"],
        chat_completion_response=response,
    )
    return WIRE.validate_json(result.model_dump_json())
