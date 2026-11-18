from litellm.llms.bedrock.embed.embedding import BedrockEmbedding
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue

from .types import TransformRequest
from .views import reply_value

UNMAPPED = "Unable to map model response to known provider format."


class Transformer(BedrockEmbedding):
    def replied(self, case: TransformRequest) -> EmbeddingResponse | None:
        return self._transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L178-L252
            case["response_list"],
            case["model"],
            case["provider"],
            case.get("is_async_invoke"),
            case.get("batch_data"),
        )


def transform_case(case: TransformRequest) -> JsonValue:
    response = Transformer().replied(case)
    if response is None:
        raise ValueError(f"{UNMAPPED} model={case['model']}")
    return reply_value(response)
