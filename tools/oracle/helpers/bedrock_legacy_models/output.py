from typing import Protocol, TypedDict

from litellm.llms.bedrock.chat.invoke_transformations.amazon_mistral_transformation import (
    AmazonMistralConfig,
)
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import ModelResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter


class MistralOutput(Protocol):
    @staticmethod
    def get_outputText(
        completion_response: dict[str, Json], model_response: ModelResponse
    ) -> str: ...


MISTRAL_OUTPUT: type[MistralOutput] = AmazonMistralConfig


class OutputCase(TypedDict):
    response: dict[str, Json]


def output(raw: Json) -> Json:
    case = TypeAdapter(OutputCase).validate_python(raw)
    model_response = ModelResponse()
    try:
        text = MISTRAL_OUTPUT.get_outputText(case["response"], model_response)
    except BedrockError as error:
        return {"error": {"status": error.status_code, "message": error.message}}
    return {"text": text, "finish_reason": model_response.choices[0].finish_reason}
