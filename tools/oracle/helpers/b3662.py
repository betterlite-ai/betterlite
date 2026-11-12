from typing import Literal, TypeAlias, TypedDict

from litellm.types.guardrails import (
    ApplyGuardrailRequest,
    BaseLitellmParams,
    ContentFilterConfigModel,
    LitellmParams,
    PatchGuardrailRequest,
)
from pydantic import BaseModel, ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

PARAM_KEYS: frozenset[str] = frozenset({"guardrail", "decision_model", "checks", "mode"})


class Input(TypedDict):
    function: Literal[
        "base_params", "litellm_params", "patch_request", "apply_request", "content_filter"
    ]
    body: dict[str, Json]


def validated(function: str, body: dict[str, Json]) -> BaseModel | None:
    match function:
        case "base_params":
            return BaseLitellmParams.model_validate(body)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L907
        case "litellm_params":
            return LitellmParams.model_validate(body)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L1192
        case "patch_request":
            return PatchGuardrailRequest.model_validate(body).litellm_params
        case "apply_request":
            return ApplyGuardrailRequest.model_validate(body)
        case _:
            return ContentFilterConfigModel.model_validate(body)


def run(input_value: Input) -> Json:
    function = input_value["function"]
    try:
        model = validated(function, input_value["body"])
    except ValidationError:
        return {"accepted": False}
    if model is None:
        return {"accepted": True, "dump": None}
    dump: dict[str, Json] = model.model_dump(mode="json", exclude_unset=function == "patch_request")
    if function == "litellm_params":
        kept = BaseLitellmParams.model_fields.keys() | PARAM_KEYS | model.model_fields_set  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/guardrails.py#L907
        dump = {key: value for key, value in dump.items() if key in kept}
    return {"accepted": True, "dump": dump}
