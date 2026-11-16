from typing_extensions import TypedDict

from .types import ORG, Input


class CallParams(TypedDict):
    api_key: str | None
    api_base: str | None
    timeout: float
    max_retries: int
    organization: str


def call_params(input_value: Input) -> CallParams:
    return {
        "api_key": input_value["api_key"],
        "api_base": input_value["api_base"],
        "timeout": 30.0,
        "max_retries": 0,
        "organization": ORG,
    }
