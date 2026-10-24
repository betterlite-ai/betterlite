import asyncio
from typing import TypeAlias, cast

import litellm
from litellm.exceptions import BadRequestError
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.llms.bedrock.common_utils import (
    AmazonBedrockGlobalConfig,
    ModelResponseIterator,
    merge_bedrock_aws_request_params,
)
from litellm.llms.bedrock.request_metadata import (
    bedrock_request_metadata_headers,
    bedrock_request_metadata_is_owned,
    merge_bedrock_invoke_headers,
    resolve_bedrock_request_metadata,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def header_pairs(value: list[Json]) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    for pair in value:
        match pair:
            case [str() as name, str() as text]:
                pairs.append((name, text))
            case _:
                raise ValueError("header pairs must be [name, value] strings")
    return pairs


def validate(metadata: Json) -> Json:
    try:
        # the pinned validator is private and takes any object, so the adapter calls it directly
        config = AmazonConverseConfig()
        config._validate_request_metadata(metadata)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportArgumentType]
    except BadRequestError as error:
        return {
            "class": type(error).__name__,
            "status": error.status_code,
            "message": error.message,
        }
    return None


async def pull(iterator: ModelResponseIterator) -> Json:
    return cast(Json, await anext(iterator))


def iterate(response: Json, calls: list[Json]) -> list[Json]:
    iterator = ModelResponseIterator(model_response=response)
    outcomes: list[Json] = []
    for call in calls:
        try:
            value = next(iterator) if call == "sync" else asyncio.run(pull(iterator))
            outcomes.append({"yield": value})
        except (StopIteration, StopAsyncIteration):
            outcomes.append("stop")
    return outcomes


def resolve(step: dict[str, Json]) -> Json:
    params = step.get("litellm_params")
    litellm_params = params if isinstance(params, dict) else None
    resolved = resolve_bedrock_request_metadata(litellm_params, step.get("caller"))
    names, pairs = bedrock_request_metadata_headers(litellm_params)
    result: dict[str, Json] = {
        "owned": bedrock_request_metadata_is_owned(),
        "resolved": None if resolved is None else [[key, text] for key, text in resolved.items()],
        "header_names": [*sorted(names)],
        "header_pairs": [[name, text] for name, text in pairs],
    }
    return result


def merge(headers: list[Json], guardrail: list[Json], params: Json) -> Json:
    names, proxy_owned = bedrock_request_metadata_headers(
        params if isinstance(params, dict) else None
    )
    original, caller_owned = dict(header_pairs(headers)), tuple(header_pairs(guardrail))
    merged = merge_bedrock_invoke_headers(original, caller_owned, proxy_owned, names)
    return [[name, text] for name, text in merged.items()]


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("fields"):
        case list() as fields:
            litellm.bedrock_request_metadata_fields = [str(field) for field in fields]
        case _:
            litellm.bedrock_request_metadata_fields = []
    match input_value.get("step"):
        case {"resolve": dict() as step}:
            return resolve(step)
        case {"validate": {"metadata": list() as metadata}}:
            return [validate(value) for value in metadata]
        case {"merge": {"headers": list() as headers, "guardrail": list() as guardrail, **rest}}:
            return merge(headers, guardrail, rest.get("litellm_params"))
        case {"aliases": {"non_default": dict() as non_default, "optional": dict() as optional}}:
            # LiteLLM types both parameters as a bare dict
            AmazonBedrockGlobalConfig().map_special_auth_params(non_default, optional)  # pyright: ignore[reportUnknownMemberType]
            return [[key, value] for key, value in optional.items()]
        case {"merge_params": {"deployment": dict() as deployment, "request": dict() as request}}:
            merged = cast(dict[str, Json], merge_bedrock_aws_request_params(deployment, request))
            return [[key, value] for key, value in merged.items()]
        case {"iterate": {"response": response, "calls": list() as calls}}:
            return iterate(response, calls)
        case step:
            raise ValueError(f"unknown bedrock_request_policy step: {step}")
