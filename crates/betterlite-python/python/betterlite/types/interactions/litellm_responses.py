from pydantic import JsonValue, PrivateAttr

from betterlite.types.llms import base as betterlite_base


class InteractionsAPIResponse(betterlite_base.BaseLiteLLMOpenAIResponseObject):
    id: str | None = None
    object: str | None = "interaction"
    model: str | None = None
    agent: str | None = None
    status: str | None = None
    created: str | None = None
    updated: str | None = None
    outputs: list[dict[str, JsonValue]] | None = None
    steps: list[dict[str, JsonValue]] | None = None
    usage: dict[str, JsonValue] | None = None

    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


# ast-grep-ignore: no-long-names
class InteractionsAPIStreamingResponse(betterlite_base.BaseLiteLLMOpenAIResponseObject):
    event_type: str | None = None
    id: str | None = None
    object: str | None = "interaction"
    model: str | None = None
    agent: str | None = None
    status: str | None = None
    created: str | None = None
    updated: str | None = None
    outputs: list[dict[str, JsonValue]] | None = None
    steps: list[dict[str, JsonValue]] | None = None
    usage: dict[str, JsonValue] | None = None
    delta: dict[str, JsonValue] | None = None
    index: int | None = None
    step: dict[str, JsonValue] | None = None
    interaction_id: str | None = None
    interaction: dict[str, JsonValue] | None = None

    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


class DeleteInteractionResult(betterlite_base.BaseLiteLLMOpenAIResponseObject):
    success: bool = True
    id: str | None = None

    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)


class CancelInteractionResult(betterlite_base.BaseLiteLLMOpenAIResponseObject):
    id: str | None = None
    status: str | None = None

    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
