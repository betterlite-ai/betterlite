import base64
import binascii
import re
from collections.abc import Mapping
from typing import Final, Protocol, TypeVar

from pydantic import JsonValue, TypeAdapter

from betterlite._router_types_params import GenericLiteLLMParams
from betterlite.types.containers.main import (
    ContainerCreateOptionalRequestParams as CreateParams,
)
from betterlite.types.containers.main import (
    ContainerListOptionalRequestParams as ListParams,
)
from betterlite.types.responses.main import DecodedResponseId

PREFIX: Final = "cntr_"
EMPTY: Final = ""
NONE_SEGMENT: Final = "None"
OPENAI: Final = "openai"
TARGETS: Final = "target_model_names"
MODEL_INFO: Final = "model_info"
ID_PATTERN: Final = re.compile(
    "^litellm:custom_llm_provider:([^;]*);model_id:([^;]*);container_id:(.+)$"
)
CREATE_KEYS: Final[tuple[str, ...]] = ("expires_after", "file_ids", "extra_headers", "extra_body")
LIST_KEYS: Final[tuple[str, ...]] = ("after", "limit", "order", "extra_headers", "extra_query")
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
CREATE_PARAMS: Final[TypeAdapter[CreateParams]] = TypeAdapter(CreateParams)
LIST_PARAMS: Final[TypeAdapter[ListParams]] = TypeAdapter(ListParams)


class ContainerConfig(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...

    def map_openai_params(
        self,
        container_create_optional_params: dict[str, JsonValue],  # ast-grep-ignore: no-long-names
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


# a response object the encode step can rename, so the original id rides on the result
class Identified(Protocol):
    id: str


IdentifiedT = TypeVar("IdentifiedT", bound=Identified)


def id_parts(provider: str | None, model_id: str | None, container: str) -> DecodedResponseId:
    return {
        "custom_llm_provider": provider, "model_id": model_id, "response_id": container,
    }


def segment(value: str) -> str | None:
    if value in (EMPTY, NONE_SEGMENT):
        return None
    return value


# ResponsesAPIRequestUtils._decode_container_id: a plain id answers itself; a bad one keeps it
def decode_container_id(container_id: str) -> DecodedResponseId:
    if not container_id.startswith(PREFIX):
        return id_parts(None, None, container_id)
    try:
        packed = container_id.replace(PREFIX, EMPTY).encode("utf-8")
        decoded = base64.b64decode(packed).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError):
        return id_parts(None, None, container_id)
    match = ID_PATTERN.match(decoded)
    if match is None:
        return id_parts(None, None, container_id)
    return id_parts(segment(match.group(1)), segment(match.group(2)), match.group(3))


# ResponsesAPIRequestUtils._build_container_id, with None written as an empty segment
def encode_container_id(
    custom_llm_provider: str | None, model_id: str | None, container_id: str,
) -> str:
    provider = EMPTY if custom_llm_provider is None else custom_llm_provider
    model = EMPTY if model_id is None else model_id
    assembled = (
        f"litellm:custom_llm_provider:{provider};model_id:{model};container_id:{container_id}"
    )
    encoded = base64.b64encode(assembled.encode("utf-8")).decode("utf-8")
    return f"{PREFIX}{encoded}"


# a managed id names its provider and model, which the caller's own values only fill in
def decode_managed_container_id_for_request(  # ast-grep-ignore: no-long-names
    container_id: str, custom_llm_provider: str, litellm_params: GenericLiteLLMParams,
) -> tuple[str, str, GenericLiteLLMParams]:
    decoded = decode_container_id(container_id)
    provider = custom_llm_provider
    discovered = decoded.get("custom_llm_provider")
    if discovered is not None and custom_llm_provider == OPENAI:
        provider = discovered
    model_id = decoded.get("model_id")
    if model_id is not None and litellm_params.get("model_id") is None:
        litellm_params["model_id"] = model_id
    response_id = decoded.get("response_id")
    if response_id is None:
        response_id = container_id
    return response_id, provider, litellm_params


# a creation or list call keeps only the keywords LiteLLM reads, and only when the caller gave them
def requested_params(
    keys: tuple[str, ...], passed_params: Mapping[str, JsonValue],
) -> dict[str, JsonValue]:
    return {
        name: passed_params[name] for name in keys if passed_params.get(name) is not None
    }


# the create call's own keywords, kept only when the caller gave them
def get_requested_container_create_optional_param(  # ast-grep-ignore: no-long-names
    passed_params: Mapping[str, JsonValue],
) -> CreateParams:
    chosen = requested_params(CREATE_KEYS, passed_params)
    return CREATE_PARAMS.validate_python(chosen)


# the list call's own keywords, kept only when the caller gave them
def get_requested_container_list_optional_param(  # ast-grep-ignore: no-long-names
    passed_params: Mapping[str, JsonValue],
) -> ListParams:
    chosen = requested_params(LIST_KEYS, passed_params)
    return LIST_PARAMS.validate_python(chosen)


# the provider config drops what it does not support and maps the rest, without dropping parameters
def get_optional_params_container_create(  # ast-grep-ignore: no-long-names
    container_provider_config: ContainerConfig,
    container_create_optional_params: CreateParams,  # ast-grep-ignore: no-long-names
) -> dict[str, JsonValue]:
    supported = container_provider_config.get_supported_openai_params()
    params = container_create_optional_params  # ast-grep-ignore: no-long-names
    filtered = {
        name: JSON.validate_python(value)
        for name, value in params.items()
        if name in supported
    }
    return container_provider_config.map_openai_params(filtered, drop_params=False)


def model_id_of(metadata: Mapping[str, JsonValue]) -> str | None:
    info = metadata.get(MODEL_INFO)
    if info is None:
        return None
    found = OBJECT.validate_python(info).get("id")
    return None if found is None else TEXT.validate_python(found)


def first_model(models: JsonValue) -> str | None:
    match models:
        case str() as names:
            return names.split(",")[0].strip()
        case [first, *_rest]:
            return str(first).strip()
        case object():
            return None


# a routed container answers an id that names its provider and model, so a follow-up call finds them
def encode_container_id_in_response(  # ast-grep-ignore: no-long-names
    response_obj: IdentifiedT, custom_llm_provider: str | None,
    litellm_metadata: Mapping[str, JsonValue] | None = None,
    extra_body: Mapping[str, JsonValue] | None = None,
) -> IdentifiedT:
    metadata: Mapping[str, JsonValue] = {} if litellm_metadata is None else litellm_metadata
    model_id = model_id_of(metadata)
    encode = model_id is not None
    if extra_body is not None and TARGETS in extra_body:
        encode = True
        if model_id is None:
            model_id = first_model(extra_body[TARGETS])
    if not encode:
        return response_obj
    response_obj.id = encode_container_id(custom_llm_provider, model_id, response_obj.id)
    return response_obj
