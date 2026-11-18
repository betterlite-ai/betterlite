from typing import TypedDict

from litellm.proxy.ui_crud_endpoints import latest_release_endpoints


class Input(TypedDict):
    body: str


class Output(TypedDict):
    new_features: int
    bug_fixes: int
    other_updates: int


def run(input_value: Input) -> Output:
    counts = latest_release_endpoints.count_release_bullets(input_value["body"])
    output: Output = {
        "new_features": counts.get("new_features", 0),
        "bug_fixes": counts.get("bug_fixes", 0),
        "other_updates": counts.get("other_updates", 0),
    }
    return output
