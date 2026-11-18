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


def unmapped_value(case: TransformRequest) -> dict[str, JsonValue]:
    message = f"{UNMAPPED} model={case['model']}"
    return {"error": {"kind": "unmapped", "message": message}}


def transform_case(case: TransformRequest) -> JsonValue:
    try:
        response = Transformer().replied(case)
    except Exception as error:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L250
        message = str(error)
        if not message.startswith(UNMAPPED):
            raise
        return {"error": {"kind": "unmapped", "message": message}}
    if response is None:
        return unmapped_value(case)
    return reply_value(response)
