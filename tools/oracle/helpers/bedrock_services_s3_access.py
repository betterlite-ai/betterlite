import os
from collections.abc import Callable, Mapping
from types import MappingProxyType
from typing import Final, Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.cloud_storage_security import should_allow_legacy_cloud_file_ids
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.common_utils import (
    BedrockError,
    CommonBatchFilesUtils,
    resolve_s3_bucket_owner,
    resolve_s3_encryption_key_id,
)
from litellm.llms.bedrock.files.handler import BedrockFilesHandler
from litellm.llms.bedrock.files.transformation import (
    _invoke_route_model,  # pyright: ignore[reportPrivateUsage]
    _resolve_managed_s3_object,  # pyright: ignore[reportPrivateUsage]
    _strip_llm_routing_prefix,  # pyright: ignore[reportPrivateUsage]
    _validate_file_id_against_configured_buckets,  # pyright: ignore[reportPrivateUsage]
    extract_s3_uri_from_file_id,
    get_configured_s3_bucket_name,
    get_configured_s3_bucket_names,
)

litellm.suppress_debug_info = True

PAIR_LENGTH: Final = 2

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Snapshot: TypeAlias = MappingProxyType[str, Json]


class AwsBaseView(Protocol):
    aws_authentication_params: list[str]
    iam_cache: object


class BatchFilesView(Protocol):
    _base_aws: AwsBaseView
    aws_authentication_params: list[str]

    def get_bedrock_model_id_from_litellm_model(self, model: str) -> str: ...

    def parse_s3_uri(self, s3_uri: str) -> tuple[str, str]: ...


class FilesHandlerView(Protocol):
    async_httpx_client: object
    aws_authentication_params: list[str]

    def _extract_s3_uri_from_file_id(self, file_id: str) -> str: ...

    def _get_configured_s3_bucket_name(self, litellm_params: Mapping[str, object]) -> str: ...


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(dict[str, Json], value)


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return cast(list[Json], value)


def text_of(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def flag_of(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def texts_of(params: Params, name: str) -> list[str]:
    return [text_of(entry, name) for entry in list_of(params.get(name), name)]


def environment_of(params: Params) -> dict[str, str]:
    environment = params.get("environment")
    if environment is None:
        return {}
    return cast(dict[str, str], object_of(environment, "environment"))


def pairs_of(value: Json, name: str) -> dict[str, Json]:
    pairs: dict[str, Json] = {}
    for entry in list_of(value, name):
        item = list_of(entry, name)
        if len(item) != PAIR_LENGTH:
            raise ValueError(f"{name} entries must be [key, value]")
        pairs[text_of(item[0], name)] = item[1]
    return pairs


def snapshot_of(params: Params, name: str) -> Snapshot | None:
    value = params.get(name)
    if value is None:
        return None
    return MappingProxyType(pairs_of(value, name))


def credentials_params(snapshot: Snapshot | None, *, trusted: bool) -> dict[str, object]:
    if snapshot is None:
        return {}
    credentials: object = snapshot if trusted else dict(snapshot)
    return {"_litellm_internal_model_credentials": credentials}


def with_request_buckets(mapping: dict[str, object], params: Params) -> dict[str, object]:
    for name in ("s3_bucket_name", "s3_output_bucket_name"):
        value = params.get(name)
        if value is not None:
            mapping[name] = value
    return mapping


def bucket_results(litellm_params: Mapping[str, object]) -> Json:
    try:
        single: Json = {"bucket": get_configured_s3_bucket_name(litellm_params)}
    except ValueError:
        single = {"failed": True}
    try:
        multiple: Json = {"buckets": list(get_configured_s3_bucket_names(litellm_params))}
    except ValueError:
        multiple = {"failed": True}
    return {"single": single, "multi": multiple}


def managed_file_id_decode(params: Params) -> Json:
    values: list[Json] = []
    for file_id in texts_of(params, "file_ids"):
        try:
            decoded = extract_s3_uri_from_file_id(file_id)
        except ValueError:
            values.append({"failed": True})
            continue
        values.append({"value": decoded})
    return {"values": values}


def configured_input_output_buckets(params: Params) -> Json:
    snapshot = snapshot_of(params, "trusted")
    mapping = with_request_buckets(credentials_params(snapshot, trusted=True), params)
    with patch.dict(os.environ, environment_of(params), clear=True):
        return bucket_results(mapping)


def configured_prefix_boundary(params: Params) -> Json:
    try:
        bucket, name = _validate_file_id_against_configured_buckets(  # pyright: ignore[reportPrivateUsage]
            text_of(params.get("s3_uri"), "s3_uri"),
            tuple(texts_of(params, "buckets")),
            flag_of(params.get("legacy"), "legacy"),
        )
    except ValueError:
        return {"failed": True}
    return {"bucket": bucket, "object": name}


def foreign_bucket_rejected(params: Params) -> Json:
    snapshot = snapshot_of(params, "trusted")
    mapping = credentials_params(snapshot, trusted=True)
    with patch.dict(os.environ, environment_of(params), clear=True):
        try:
            bucket, name = _resolve_managed_s3_object(  # pyright: ignore[reportPrivateUsage]
                text_of(params.get("file_id"), "file_id"), mapping
            )
        except BedrockError as error:
            return {"error": {"status": error.status_code}}
        except ValueError:
            return {"error": {"status": None}}
    return {"bucket": bucket, "object": name}


def legacy_id_opt_in(params: Params) -> Json:
    snapshot = snapshot_of(params, "trusted")
    mapping = credentials_params(snapshot, trusted=True)
    opt_in = should_allow_legacy_cloud_file_ids(mapping)
    try:
        bucket, name = _validate_file_id_against_configured_buckets(  # pyright: ignore[reportPrivateUsage]
            text_of(params.get("s3_uri"), "s3_uri"),
            (text_of(params.get("bucket"), "bucket"),),
            opt_in,
        )
    except ValueError:
        return {"opt_in": opt_in, "authorized": {"failed": True}}
    return {"opt_in": opt_in, "authorized": {"bucket": bucket, "object": name}}


def request_credentials_rejected(params: Params) -> Json:
    trusted = with_request_buckets(
        credentials_params(snapshot_of(params, "trusted"), trusted=True), params
    )
    plain = with_request_buckets(
        credentials_params(snapshot_of(params, "plain"), trusted=False), params
    )
    request = with_request_buckets({}, params)
    malformed = credentials_params(snapshot_of(params, "malformed"), trusted=True)
    with patch.dict(os.environ, environment_of(params), clear=True):
        return {
            "trusted": bucket_results(trusted),
            "plain": bucket_results(plain),
            "request": bucket_results(request),
            "malformed": bucket_results(malformed),
        }


def setting_map(params: Params, name: str) -> dict[str, Json] | None:
    value = params.get(name)
    if value is None:
        return None
    return object_of(value, name)


def kms_owner_precedence(params: Params) -> Json:
    litellm_params = object_of(params.get("litellm_params"), "litellm_params")
    optional_params = setting_map(params, "optional_params")
    with patch.dict(os.environ, environment_of(params), clear=True):
        return {
            "encryption_key_id": resolve_s3_encryption_key_id(
                litellm_params=litellm_params, optional_params=optional_params
            ),
            "bucket_owner": resolve_s3_bucket_owner(
                litellm_params=litellm_params, optional_params=optional_params
            ),
        }


def s3_model_helpers(params: Params) -> Json:
    models = texts_of(params, "models")
    with patch.dict(os.environ, environment_of(params), clear=True):
        utils = cast(BatchFilesView, CommonBatchFilesUtils())
        model_ids = [utils.get_bedrock_model_id_from_litellm_model(model) for model in models]
        stripped = [
            _strip_llm_routing_prefix(model)  # pyright: ignore[reportPrivateUsage]
            for model in models
        ]
        invoked = [
            _invoke_route_model(model)  # pyright: ignore[reportPrivateUsage]
            for model in models
        ]
    base = utils._base_aws  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    shared: object = BaseAWSLLM._shared_iam_cache  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    return cast(
        Json,
        {
            "authentication_params": list(base.aws_authentication_params),
            "shared_cache": base.iam_cache is shared,
            "model_ids": model_ids,
            "stripped": stripped,
            "invoked": invoked,
        },
    )


def plain_s3_uri_parser(params: Params) -> Json:
    utils = cast(BatchFilesView, CommonBatchFilesUtils())
    values: list[Json] = []
    for uri in texts_of(params, "uris"):
        try:
            bucket, key = utils.parse_s3_uri(uri)
        except ValueError:
            values.append({"failed": True})
            continue
        values.append({"bucket": bucket, "key": key})
    return {"values": values}


def files_handler_initialization(params: Params) -> Json:
    snapshot = snapshot_of(params, "trusted")
    mapping = credentials_params(snapshot, trusted=True)
    extracted: list[Json] = []
    with patch.dict(os.environ, environment_of(params), clear=True):
        handler = cast(FilesHandlerView, BedrockFilesHandler())
        for file_id in texts_of(params, "file_ids"):
            try:
                extracted.append(
                    {"value": handler._extract_s3_uri_from_file_id(file_id)}  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                )
            except ValueError:
                extracted.append({"failed": True})
        try:
            bucket: Json = {
                "bucket": handler._get_configured_s3_bucket_name(mapping)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            }
        except ValueError:
            bucket = {"failed": True}
    return {
        "authentication_params": list(handler.aws_authentication_params),
        "client": handler.async_httpx_client is not None,
        "extract": extracted,
        "bucket": bucket,
    }


HANDLERS: Final[dict[str, Callable[[Params], Json]]] = {
    "managed_file_id_decode": managed_file_id_decode,
    "configured_input_output_buckets": configured_input_output_buckets,
    "configured_prefix_boundary": configured_prefix_boundary,
    "foreign_bucket_rejected": foreign_bucket_rejected,
    "legacy_id_opt_in": legacy_id_opt_in,
    "request_credentials_rejected": request_credentials_rejected,
    "kms_owner_precedence": kms_owner_precedence,
    "s3_model_helpers": s3_model_helpers,
    "plain_s3_uri_parser": plain_s3_uri_parser,
    "files_handler_initialization": files_handler_initialization,
}


def run(input_value: Json) -> Json:
    params = object_of(input_value, "input")
    operation = text_of(params.get("operation"), "operation")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation {operation}")
    return handler(params)
