from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue

from .support import JSON


class RecordView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    object: str
    embedding: list[JsonValue] | str
    index: int


class DetailsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    image_count: int


class UsageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: DetailsView | None


class ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    object: str
    data: list[RecordView]
    model: str | None
    usage: UsageView


# the response objects keep fields betterlite does not emit, so the view is the shared record
def response_out(response: EmbeddingResponse) -> JsonValue:
    view = ResponseView.model_validate(response)
    return JSON.validate_json(view.model_dump_json())
