from typing import TypedDict

from litellm.proxy.common_utils.path_utils import safe_filename


class Input(TypedDict):
    filename: str


def run(input_value: Input) -> str:
    return safe_filename(input_value["filename"])
