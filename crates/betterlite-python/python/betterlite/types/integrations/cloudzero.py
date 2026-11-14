from typing import TypeAlias

from pydantic import JsonValue


class CBFRecord(dict[str, JsonValue]):
    pass


CBFRecordDict: TypeAlias = dict[str, JsonValue]
