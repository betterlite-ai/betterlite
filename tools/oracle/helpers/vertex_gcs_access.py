import os
from types import MappingProxyType
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.litellm_core_utils.cloud_storage_security import (
    sanitize_cloud_object_component,
    sanitize_cloud_object_path,
)
from litellm.llms.vertex_ai.files.transformation import VertexAIFilesConfig

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Operation: TypeAlias = Literal[
    "configured_prefix",
    "encoded_traversal",
    "foreign_bucket",
    "terminal_slash",
    "percent_reencoding",
    "trusted_legacy",
    "forged_trust",
    "filename_sanitization",
    "model_path",
]
TRUSTED_KEY = "_litellm_internal_model_credentials"


class Input(TypedDict):
    operation: Operation
    params: dict[str, str]
    environment: dict[str, str]
    trusted: list[tuple[str, Json]]
    plain: list[tuple[str, Json]]
    file_ids: list[str]
    filenames: list[str | None]
    model_paths: list[str | None]


def snapshot_of(pairs: list[tuple[str, Json]]) -> MappingProxyType[str, Json]:
    values: dict[str, Json] = dict(pairs)
    return MappingProxyType(values)


def litellm_params_of(input_value: Input, trusted: list[tuple[str, Json]]) -> Params:
    params: Params = {}
    for name, value in input_value["params"].items():
        params[name] = value
    for plain_name, plain_value in input_value["plain"]:
        params[plain_name] = plain_value
    if trusted:
        # LiteLLM trusts only a MappingProxyType snapshot, never a plain request mapping
        params[TRUSTED_KEY] = cast(Json, snapshot_of(trusted))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/cloud_storage_security.py#L121
    return params


def parse_results(
    config: VertexAIFilesConfig, input_value: Input, trusted: list[tuple[str, Json]]
) -> Json:
    params = litellm_params_of(input_value, trusted)
    optional_params: Params = {}
    results: list[Json] = []
    for file_id in input_value["file_ids"]:
        try:
            response = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/transformation.py#L1153
                tuple[str, Params],
                config.transform_retrieve_file_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/transformation.py#L1153
                    file_id, optional_params, params
                ),
            )
        except ValueError:
            results.append({"failed": True})
        else:
            results.append({"url": response[0]})
    return {"results": results}


def run(input_value: Input) -> Json:
    for name, value in input_value["environment"].items():
        os.environ[name] = value
    if input_value["operation"] == "filename_sanitization":
        return {
            "components": [
                sanitize_cloud_object_component(filename, "file")
                for filename in input_value["filenames"]
            ]
        }
    if input_value["operation"] == "model_path":
        return {
            "paths": [
                sanitize_cloud_object_path(model_path, "model")
                for model_path in input_value["model_paths"]
            ]
        }
    trusted: list[tuple[str, Json]] = []
    if input_value["operation"] == "trusted_legacy":
        trusted = input_value["trusted"]
    config = VertexAIFilesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/files/transformation.py#L868
    return parse_results(config, input_value, trusted)
