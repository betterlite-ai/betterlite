import json
from collections.abc import Callable, Mapping
from typing import Final, Literal, TypeAlias, TypedDict, cast

from litellm.proxy import utils as proxy_utils
from pydantic import JsonValue, TypeAdapter

POLICY_STATE: Final = proxy_utils._policy_state_metadata  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L547
MERGE_WRITES: Final = cast(Callable[[dict[str, JsonValue], Mapping[str, object]], None], proxy_utils._merge_pipeline_metadata_writes)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L622
MERGE_GUARDRAILS: Final = cast(Callable[[dict[str, JsonValue], JsonValue], dict[str, JsonValue]], proxy_utils._merge_guardrails_with_existing)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L8100

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
GUARDRAILS: Final = TypeAdapter(list[JsonValue])


class PolicyStateCall(TypedDict):
    function: Literal["policy_state"]
    data: dict[str, JsonValue]


class MergeWritesCall(TypedDict):
    function: Literal["merge_metadata_writes"]
    data: dict[str, JsonValue]
    modified: dict[str, JsonValue]


class MergeGuardrailsCall(TypedDict):
    function: Literal["merge_guardrails"]
    data: dict[str, JsonValue]
    model_level_guardrails: JsonValue


Call: TypeAlias = PolicyStateCall | MergeWritesCall | MergeGuardrailsCall


def run(input_value: Call) -> JsonValue:
    match input_value["function"]:
        case "policy_state":
            return policy_state_answer(input_value["data"])
        case "merge_metadata_writes":
            return merge_writes_answer(input_value["data"], input_value["modified"])
        case "merge_guardrails":
            return merge_answer(input_value["data"], input_value["model_level_guardrails"])


def policy_state_answer(data: dict[str, JsonValue]) -> JsonValue:
    state = POLICY_STATE(data)
    if len(state) == 0:
        return None
    return OBJECT.validate_python(state)


def merge_writes_answer(data: dict[str, JsonValue], modified: dict[str, JsonValue]) -> JsonValue:
    MERGE_WRITES(data, modified)
    return data


def merge_answer(data: dict[str, JsonValue], model_level_guardrails: JsonValue) -> JsonValue:
    merged = MERGE_GUARDRAILS(data, model_level_guardrails)
    return canonical_guardrails(merged)


def canonical_guardrails(merged: dict[str, JsonValue]) -> dict[str, JsonValue]:
    metadata = OBJECT.validate_python(merged["metadata"])
    guardrails = GUARDRAILS.validate_python(metadata["guardrails"])
    metadata["guardrails"] = sorted(guardrails, key=canonical)
    merged["metadata"] = metadata
    return merged


def canonical(value: JsonValue) -> str:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    )
