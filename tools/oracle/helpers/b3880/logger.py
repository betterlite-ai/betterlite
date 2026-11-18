from typing import Final

from litellm.integrations.prometheus import PrometheusLogger
from pydantic import JsonValue
from typing_extensions import override


class LabelLogger(PrometheusLogger):
    @override
    def __init__(self, **_kwargs: JsonValue) -> None:
        pass


LOGGER: Final[PrometheusLogger] = LabelLogger()
