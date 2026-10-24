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


def mapping(value: Json) -> dict[str, Json]:
    return value if isinstance(value, dict) else {}


def items(value: Json) -> list[Json]:
    return value if isinstance(value, list) else []


def header_pairs(value: Json) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    for pair in items(value):
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


def merge(step: dict[str, Json]) -> Json:
    params = step.get("litellm_params")
    names, proxy_owned = bedrock_request_metadata_headers(
        params if isinstance(params, dict) else None
    )
    headers = dict(header_pairs(step.get("headers")))
    guardrail = tuple(header_pairs(step.get("guardrail")))
    merged = merge_bedrock_invoke_headers(headers, guardrail, proxy_owned, names)
    return [[name, text] for name, text in merged.items()]


def run(input_value: dict[str, Json]) -> Json:
    litellm.bedrock_request_metadata_fields = [
        str(field) for field in items(input_value.get("fields"))
    ]
    steps = list(mapping(input_value.get("step")).items())
    if len(steps) != 1:
        raise ValueError("bedrock_request_policy needs exactly one step")
    name, body = steps[0]
    step = mapping(body)
    match name:
        case "resolve":
            return resolve(step)
        case "validate":
            return [validate(metadata) for metadata in items(step.get("metadata"))]
        case "merge":
            return merge(step)
        case "aliases":
            optional = mapping(step.get("optional"))
            non_default = mapping(step.get("non_default"))
            # LiteLLM types both parameters as a bare dict
            AmazonBedrockGlobalConfig().map_special_auth_params(non_default, optional)  # pyright: ignore[reportUnknownMemberType]
            return [[key, value] for key, value in optional.items()]
        case "merge_params":
            deployment, request = mapping(step.get("deployment")), mapping(step.get("request"))
            merged = cast(dict[str, Json], merge_bedrock_aws_request_params(deployment, request))
            return [[key, value] for key, value in merged.items()]
        case "iterate":
            return iterate(step.get("response"), items(step.get("calls")))
        case _:
            raise ValueError(f"unknown bedrock_request_policy step: {name}")
