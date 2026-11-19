from collections import Counter
from typing import TypedDict

from litellm.proxy.ui_crud_endpoints import latest_release_endpoints


class Input(TypedDict):
    body: str


class Output(TypedDict):
    new_features: int
    bug_fixes: int
    other_updates: int


def run(input_value: Input) -> Output:
    counted = Counter(latest_release_endpoints.count_release_bullets(input_value["body"]))
    output: Output = {
        "new_features": counted["new_features"],
        "bug_fixes": counted["bug_fixes"],
        "other_updates": counted["other_updates"],
    }
    return output
