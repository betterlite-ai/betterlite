from typing import TypeAlias

from litellm.litellm_core_utils.url_utils import assert_same_origin

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    expected = {"candidate_url", "expected_url"}
    if set(input_value) != expected:
        raise ValueError("url_origin input fields must be candidate_url and expected_url")
    candidate_url = input_value["candidate_url"]
    expected_url = input_value["expected_url"]
    if not isinstance(candidate_url, str):
        raise ValueError("candidate_url must be a string")
    if not isinstance(expected_url, str):
        raise ValueError("expected_url must be a string")
    assert_same_origin(candidate_url, expected_url)
    return True
