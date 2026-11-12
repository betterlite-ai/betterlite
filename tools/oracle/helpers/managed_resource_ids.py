from collections.abc import Callable, Mapping, Sequence
from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.llms.base_llm.managed_resources import utils
from litellm.types.utils import LlmProviders

Json: TypeAlias = None | bool | int | float | str | Sequence["Json"] | Mapping[str, "Json"]


class LlmProviderInput(TypedDict):
    llm_provider: str


class ProviderCall(TypedDict):
    function: Literal["resolve_provider"]
    provider: str | LlmProviderInput | None


class DetectCall(TypedDict):
    function: Literal["detect"]
    value: str | int | None
    prefix: NotRequired[str]


class ValueCall(TypedDict):
    function: Literal[
        "decode",
        "target_model_names",
        "resource_type",
        "unified_uuid",
        "model_id",
        "provider_resource_id",
        "parse",
    ]
    value: str | int | None


class Input(TypedDict):
    calls: list[ProviderCall | DetectCall | ValueCall]


READERS: dict[str, Callable[[str], Json]] = {
    "decode": utils.decode_unified_id,
    "target_model_names": utils.extract_target_model_names_from_unified_id,
    "resource_type": utils.extract_resource_type_from_unified_id,
    "unified_uuid": utils.extract_unified_uuid_from_unified_id,
    "model_id": utils.extract_model_id_from_unified_id,
    "provider_resource_id": utils.extract_provider_resource_id_from_unified_id,
    "parse": utils.parse_unified_id,  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/managed_resources/utils.py#L358
}


def run(input_value: Input) -> list[Json]:
    results: list[Json] = []
    for call in input_value["calls"]:
        if call["function"] == "resolve_provider":
            match call["provider"]:
                case str() | None as name:
                    results.append(utils.resolve_passthrough_managed_id_provider(name))
                case named:
                    results.append(
                        utils.resolve_passthrough_managed_id_provider(
                            LlmProviders(named["llm_provider"])
                        )
                    )
            continue
        # the readers guard with isinstance(str), and cases send other JSON values on purpose
        value: str = call["value"]  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/managed_resources/utils.py#L56
        if call["function"] != "detect":
            results.append(READERS[call["function"]](value))
        elif "prefix" in call:
            results.append(utils.is_base64_encoded_unified_id(value, call["prefix"]))
        else:
            results.append(utils.is_base64_encoded_unified_id(value))
    return results
