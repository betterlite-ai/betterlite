from collections.abc import Callable, Mapping
from typing import Annotated, Final

from pydantic import AfterValidator, Field, JsonValue, SkipValidation, TypeAdapter


def _validate_positive_router_weights(  # ast-grep-ignore: no-long-names
    weights: Mapping[str, Mapping[str, float]],
) -> Mapping[str, Mapping[str, float]]:
    if any(
        group and not any(weight > 0 for weight in group.values()) for group in weights.values()
    ):
        raise ValueError("Each nonempty weights group must contain at least one positive weight")
    return weights


RouterWeightIdentifier = Annotated[str, Field(strict=True, min_length=1, pattern=r"\S")]
RouterWeight = Annotated[float, Field(strict=True, ge=0, allow_inf_nan=False)]
RouterWeights = Annotated[
    dict[RouterWeightIdentifier, dict[RouterWeightIdentifier, RouterWeight]],
    AfterValidator(_validate_positive_router_weights),  # ast-grep-ignore: no-long-names
]
_ROUTER_WEIGHTS_ADAPTER: Final[TypeAdapter[RouterWeights | None]] = TypeAdapter(
    RouterWeights | None
)
_ROUTER_SETTINGS_DICT_ADAPTER: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(
    dict[str, SkipValidation[JsonValue]]
)

validate_router_weights: Final[Callable[[JsonValue], RouterWeights | None]] = (
    _ROUTER_WEIGHTS_ADAPTER.validate_python
)


def validate_router_settings_dict(value: JsonValue) -> dict[str, JsonValue]:
    settings: Final = _ROUTER_SETTINGS_DICT_ADAPTER.validate_python(value)
    validate_router_weights(settings.get("weights"))
    return settings
