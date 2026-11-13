from typing import TypeAlias, cast

from litellm.litellm_core_utils.url_utils import is_url_destination_allowed_by_host

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    expected = {"url", "allowed_hosts"}
    if set(input_value) != expected:
        raise ValueError("url_destination input fields must be url and allowed_hosts")
    url = input_value["url"]
    allowed_hosts = input_value["allowed_hosts"]
    if not isinstance(url, str):
        raise ValueError("url must be a string")
    if not isinstance(allowed_hosts, list):
        raise ValueError("allowed_hosts must be a list of strings")
    hosts = cast(list[str], allowed_hosts)
    return is_url_destination_allowed_by_host(url, hosts)
