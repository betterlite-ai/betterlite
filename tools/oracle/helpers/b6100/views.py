from litellm.litellm_core_utils.core_helpers import RESPONSE_COST_HEADER
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import BaseModel, Field, JsonValue

from .wire import Object


class HiddenParams(BaseModel):
    additional_headers: Object = Field(default_factory=Object)


class HiddenCarrier(BaseModel):
    params: HiddenParams = Field(alias="_hidden_params", default_factory=HiddenParams)


class CallDetails(BaseModel):
    response_cost: JsonValue = None


class RecordedLogging(BaseModel):
    model_call_details: CallDetails


class ChunkUsage(BaseModel):
    cost: JsonValue = None


class ChunkRecord(BaseModel):
    usage: ChunkUsage | None = None


def hidden_cost(response: ModelResponse | EmbeddingResponse | ResponsesAPIResponse) -> JsonValue:
    params = HiddenCarrier.model_validate(response, from_attributes=True).params
    return params.additional_headers.get(RESPONSE_COST_HEADER)
