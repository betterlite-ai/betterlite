from collections.abc import Mapping
from typing import ClassVar, Literal, cast

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.json_types import ValidationError
from betterlite.types.utils.usage_details import (
    CompletionTokensDetailsWrapper,
    PromptTokensDetailsWrapper,
    ServerToolUse,
    int_value,
)
from betterlite.types.utils.values import StoredValue

DetailKind = Literal["completion", "prompt", "tool_use"]


class Usage(ResponseObject):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    completion_tokens_details: CompletionTokensDetailsWrapper | None
    prompt_tokens_details: PromptTokensDetailsWrapper | None
    server_tool_use: ServerToolUse | None
    cost: float | None
    _cache_creation_input_tokens: int
    _cache_read_input_tokens: int
    defaults: ClassVar[dict[str, StoredValue]] = {
        "completion_tokens_details": None,
        "prompt_tokens_details": None,
        "server_tool_use": None,
        "cost": None,
        "_cache_creation_input_tokens": 0,
        "_cache_read_input_tokens": 0,
    }

    @override
    def __init__(
        self,
        prompt_tokens: StoredValue = None,
        completion_tokens: StoredValue = None,
        total_tokens: StoredValue = None,
        reasoning_tokens: StoredValue = None,
        prompt_tokens_details: StoredValue = None,
        completion_tokens_details: StoredValue = None,
        server_tool_use: StoredValue = None,
        cost: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        details = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1891-L1894
            "CompletionTokensDetailsWrapper | None",
            detail_value(completion_tokens_details, "completion", "completion_tokens_details"),
        )
        if reasoning_tokens is not None:
            details = reasoning_details(details, reasoning_tokens, completion_tokens)
        prompt_details = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1924-L1930
            "PromptTokensDetailsWrapper | None",
            detail_value(prompt_tokens_details, "prompt", "prompt_tokens_details"),
        )
        prompt_details = cached_from_params(prompt_details, params)
        super().__init__(
            prompt_tokens=token_count(prompt_tokens, "prompt_tokens"),
            completion_tokens=token_count(completion_tokens, "completion_tokens"),
            total_tokens=token_count(total_tokens, "total_tokens"),
            completion_tokens_details=details,
            prompt_tokens_details=prompt_details,
        )
        self._load_cache_tokens(params)
        if server_tool_use is None:
            del self.server_tool_use
        else:
            self.server_tool_use = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1962-L1963
                "ServerToolUse",
                detail_value(server_tool_use, "tool_use", "server_tool_use"),
            )
        if cost is None:
            del self.cost
        else:
            self.cost = cost_number(cost)
        for name, item in params.items():
            self[name] = item

    def _load_cache_tokens(self, params: Mapping[str, StoredValue]) -> None:
        created = int_param(params, "cache_creation_input_tokens")
        if created is not None:
            self._cache_creation_input_tokens = created
        read = int_param(params, "cache_read_input_tokens")
        if read is not None:
            self._cache_read_input_tokens = read
        hit = int_param(params, "prompt_cache_hit_tokens")
        if hit is not None:
            self._cache_read_input_tokens = hit


def int_param(params: Mapping[str, StoredValue], name: str) -> int | None:
    if name not in params:
        return None
    return int_value(params[name])


def cached_from_params(
    details: PromptTokensDetailsWrapper | None,
    params: Mapping[str, StoredValue],
) -> PromptTokensDetailsWrapper | None:
    targets: tuple[tuple[Literal["cached", "write"], int | None], ...] = (
        ("cached", int_param(params, "prompt_cache_hit_tokens")),
        ("cached", int_param(params, "cache_read_input_tokens")),
        ("write", int_param(params, "cache_creation_input_tokens")),
    )
    for kind, value in targets:
        if value is not None:
            details = token_details(details, value, kind)
    return details


def token_count(value: StoredValue, name: str) -> int:
    match value:
        case int():
            return value
        case None:
            return 0
        case _other:
            raise ValidationError("Usage", (name,), "expected an integer")


def cost_number(value: StoredValue) -> float:
    match value:
        case int() | float():
            return float(value)
        case _other:
            raise ValidationError("Usage", ("cost",), "expected a number")


def detail_value(value: StoredValue, kind: DetailKind, name: str) -> StoredValue:
    match value:
        case None:
            return None
        case CompletionTokensDetailsWrapper() if kind == "completion":
            return value
        case PromptTokensDetailsWrapper() if kind == "prompt":
            return value
        case ServerToolUse() if kind == "tool_use":
            return value
        case Mapping():
            return build_detail(value, kind)
        case _other:
            raise ValidationError("Usage", (name,), "wrong type")


def build_detail(value: Mapping[str, StoredValue], kind: DetailKind) -> StoredValue:
    if kind == "completion":
        return CompletionTokensDetailsWrapper(**value)
    if kind == "prompt":
        return PromptTokensDetailsWrapper(**value)
    return ServerToolUse(**value)


def reasoning_details(
    details: CompletionTokensDetailsWrapper | None,
    reasoning_tokens: StoredValue,
    completion_tokens: StoredValue,
) -> CompletionTokensDetailsWrapper:
    reasoning = token_count(reasoning_tokens, "reasoning_tokens")
    wrapper = CompletionTokensDetailsWrapper() if details is None else details
    if wrapper.reasoning_tokens is None:
        wrapper.reasoning_tokens = reasoning
    if wrapper.text_tokens is None and completion_tokens is not None:
        calculated = token_count(completion_tokens, "completion_tokens") - reasoning
        if wrapper.image_tokens is not None:
            calculated -= wrapper.image_tokens
        if wrapper.audio_tokens is not None:
            calculated -= wrapper.audio_tokens
        wrapper.text_tokens = max(0, calculated)
    return wrapper


def token_details(
    details: PromptTokensDetailsWrapper | None,
    value: int,
    kind: Literal["cached", "write"],
) -> PromptTokensDetailsWrapper:
    target = PromptTokensDetailsWrapper() if details is None else details
    if kind == "cached":
        target.cached_tokens = value
    else:
        target.cache_write_tokens = value
    return target


def text_tokens_without_nested_reasoning(  # ast-grep-ignore: no-long-names
    completion_tokens: int,
    text_tokens: int,
    reasoning_tokens: int,
    other_modality_tokens: int,
) -> int:
    reported_total = text_tokens + reasoning_tokens + other_modality_tokens
    nested_reasoning = min(
        reasoning_tokens,
        text_tokens,
        max(reported_total - completion_tokens, 0),
    )
    return text_tokens - nested_reasoning
