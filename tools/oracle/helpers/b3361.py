from typing import Final, Literal, NotRequired, Protocol, TypedDict

from litellm.containers.utils import (
    ContainerRequestUtils,
    decode_managed_container_id_for_request,
)
from litellm.llms.base_llm.containers.transformation import BaseContainerConfig
from litellm.types.containers.main import (
    ContainerCreateOptionalRequestParams as CreateOptionalParams,
    ContainerListOptionalRequestParams as ListOptionalParams,
    DeleteContainerResult,
)
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class ContainerRequests(Protocol):
    @staticmethod
    def get_requested_container_create_optional_param(  # ast-grep-ignore: no-long-names
        passed_params: dict[str, JsonValue],
    ) -> CreateOptionalParams: ...

    @staticmethod
    def get_optional_params_container_create(  # ast-grep-ignore: no-long-names
        container_provider_config: BaseContainerConfig,
        container_create_optional_params: CreateOptionalParams,  # ast-grep-ignore: no-long-names
    ) -> dict[str, JsonValue]: ...

    @staticmethod
    def get_requested_container_list_optional_param(  # ast-grep-ignore: no-long-names
        passed_params: dict[str, JsonValue],
    ) -> ListOptionalParams: ...


REQUESTS: Final[ContainerRequests] = ContainerRequestUtils


class DecodedParams(TypedDict):
    model_id: NotRequired[str | None]


DECODED: Final = TypeAdapter(DecodedParams)


class DecodeCall(TypedDict):
    function: Literal["decode_managed_id"]
    container_id: str
    custom_llm_provider: str
    litellm_params: dict[str, JsonValue]


class CreateOptionalCall(TypedDict):
    function: Literal["create_optional_params"]
    passed_params: dict[str, JsonValue]


class FilterCreateCall(TypedDict):
    function: Literal["create_params"]
    provider: str
    passed_params: dict[str, JsonValue]


class ListOptionalCall(TypedDict):
    function: Literal["list_optional_params"]
    passed_params: dict[str, JsonValue]


class EncodeCall(TypedDict):
    function: Literal["encode_container_id"]
    container_id: str
    custom_llm_provider: str | None
    litellm_metadata: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None


class ProviderConfigCall(TypedDict):
    function: Literal["provider_config"]
    custom_llm_provider: str


Call = (
    DecodeCall
    | CreateOptionalCall
    | FilterCreateCall
    | ListOptionalCall
    | EncodeCall
    | ProviderConfigCall
)


def decode_answer(call: DecodeCall) -> dict[str, JsonValue]:
    params = GenericLiteLLMParams.model_validate(call["litellm_params"])
    original, provider, updated = decode_managed_container_id_for_request(
        call["container_id"], call["custom_llm_provider"], params
    )
    decoded = DECODED.validate_json(updated.model_dump_json())
    return {
        "container_id": original,
        "custom_llm_provider": provider,
        "model_id": decoded.get("model_id"),
    }


def create_optional_answer(call: CreateOptionalCall) -> dict[str, JsonValue]:
    optional = REQUESTS.get_requested_container_create_optional_param(
        call["passed_params"]
    )
    return {"params": JSON.validate_python(optional)}


def filter_create_answer(call: FilterCreateCall) -> dict[str, JsonValue]:
    config = ProviderConfigManager.get_provider_container_config(
        LlmProviders(call["provider"])
    )
    if config is None:
        raise ValueError(f"{call['provider']} is not a known provider")
    optional = REQUESTS.get_requested_container_create_optional_param(
        call["passed_params"]
    )
    params = REQUESTS.get_optional_params_container_create(config, optional)
    return {"params": params}


def list_optional_answer(call: ListOptionalCall) -> dict[str, JsonValue]:
    optional = REQUESTS.get_requested_container_list_optional_param(
        call["passed_params"]
    )
    return {"params": JSON.validate_python(optional)}


def encode_answer(call: EncodeCall) -> dict[str, JsonValue]:
    result = DeleteContainerResult(
        id=call["container_id"], object="container.deleted", deleted=True
    )
    encoded = ContainerRequestUtils.encode_container_id_in_response(
        result,
        call["custom_llm_provider"],
        call["litellm_metadata"],
        call["extra_body"],
    )
    return {"id": encoded.id}


def provider_config_answer(call: ProviderConfigCall) -> dict[str, JsonValue]:
    config = ProviderConfigManager.get_provider_container_config(
        LlmProviders(call["custom_llm_provider"])
    )
    name = None if config is None else type(config).__name__
    return {"config": name}


def run(input_value: Call) -> JsonValue:
    if input_value["function"] == "decode_managed_id":
        return decode_answer(input_value)
    if input_value["function"] == "create_optional_params":
        return create_optional_answer(input_value)
    if input_value["function"] == "create_params":
        return filter_create_answer(input_value)
    if input_value["function"] == "list_optional_params":
        return list_optional_answer(input_value)
    if input_value["function"] == "encode_container_id":
        return encode_answer(input_value)
    return provider_config_answer(input_value)
