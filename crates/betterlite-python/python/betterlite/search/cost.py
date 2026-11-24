from collections.abc import Mapping
from typing import Annotated, Final, Literal, TypeAlias

from pydantic import BaseModel, ConfigDict, Field, TypeAdapter

from betterlite import exceptions
from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallValue, to_call_payload
from betterlite._native import NativeError, NativeSearchSession


class CostOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    outcome: Literal["cost"]
    input_cost: float
    output_cost: float


class UnmappedOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    outcome: Literal["unmapped"]
    message: str


CostAnswer: TypeAlias = Annotated[CostOutcome | UnmappedOutcome, Field(discriminator="outcome")]
COST_ANSWER: Final[TypeAdapter[CostAnswer]] = TypeAdapter(CostAnswer)
NOT_MAPPED: Final = (
    "This model isn't mapped yet. model={model}, custom_llm_provider={provider}. "
    "Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
)


# search_provider_cost_per_query: the pair LiteLLM returns, read from the cost map in Rust
def search_provider_cost_per_query(
    model: str,
    custom_llm_provider: str | None = None,
    number_of_queries: int = 1,
    optional_params: Mapping[str, CallValue] | None = None,
) -> tuple[float, float]:
    values: dict[str, CallValue] = {"model": model, "number_of_queries": number_of_queries}
    if custom_llm_provider is not None:
        values["custom_llm_provider"] = custom_llm_provider
    if optional_params is not None:
        values["optional_params"] = dict(optional_params)
    payload = to_call_payload(values)
    session = NativeSearchSession("{}")
    try:
        answer = session.cost(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)
    outcome = COST_ANSWER.validate_json(answer)
    match outcome:
        case CostOutcome():
            return (outcome.input_cost, outcome.output_cost)
        case UnmappedOutcome():
            raise exceptions.ModelNotMappedError(
                NOT_MAPPED.format(model=model, provider=custom_llm_provider)
            )
