from typing import Literal, TypedDict, cast

from litellm.constants import CLI_SSO_CLAIM_MAP
from litellm.proxy.management_endpoints import ui_sso
from litellm.proxy.management_endpoints.types import CustomOpenID
from pydantic import JsonValue, TypeAdapter


class Envelope(TypedDict):
    function: str


class BuildCall(TypedDict):
    function: Literal["build"]
    claim_map: str
    identity: dict[str, JsonValue]


class MergeCall(TypedDict):
    function: Literal["merge"]
    existing_metadata: dict[str, JsonValue]
    attribution_metadata: dict[str, JsonValue]


class FlattenCall(TypedDict):
    function: Literal["flatten"]
    attribution_metadata: dict[str, JsonValue]


class PollCall(TypedDict):
    function: Literal["poll"]
    session: dict[str, JsonValue]


ENVELOPE = TypeAdapter(Envelope)
BUILD = TypeAdapter(BuildCall)
MERGE = TypeAdapter(MergeCall)
FLATTEN = TypeAdapter(FlattenCall)
POLL = TypeAdapter(PollCall)
IDENTITY = TypeAdapter(CustomOpenID)
JSON = TypeAdapter(JsonValue)


def build(call: BuildCall) -> JsonValue:
    identity = IDENTITY.validate_python(call["identity"])
    return JSON.validate_python(ui_sso.build_cli_sso_attribution_metadata(identity))


def merge(call: MergeCall) -> JsonValue:
    existing = cast("dict[str, object]", call["existing_metadata"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L558-L560
    attribution = cast("dict[str, object]", call["attribution_metadata"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L558-L560
    merged = ui_sso._merge_cli_sso_attribution_metadata(existing, attribution)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L558-L579
    return JSON.validate_python(merged)


def flatten(call: FlattenCall) -> JsonValue:
    flattened = ui_sso._flatten_cli_sso_metadata_for_poll(call["attribution_metadata"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L512-L527
    return JSON.validate_python(flattened)


def poll(call: PollCall) -> JsonValue:
    projected = ui_sso._cli_poll_attribution_metadata_from_session(call["session"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L615-L621
    return JSON.validate_python(projected)


def run(input_value: JsonValue) -> JsonValue:
    function = ENVELOPE.validate_python(input_value)["function"]
    if function == "build":
        call = BUILD.validate_python(input_value)
        if call["claim_map"] != CLI_SSO_CLAIM_MAP:
            raise ValueError("the case environment and the claim map input disagree")
        return build(call)
    if function == "merge":
        return merge(MERGE.validate_python(input_value))
    if function == "flatten":
        return flatten(FLATTEN.validate_python(input_value))
    if function == "poll":
        return poll(POLL.validate_python(input_value))
    raise ValueError(f"unknown cli_attribution function: {function}")
