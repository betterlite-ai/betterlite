from typing import TypeAlias

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
PAIR_SIZE = 2


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    record = input_value.get("record")
    if operation == "record_to_dict":
        if isinstance(record, dict):
            return record
        if isinstance(record, list):
            result: dict[str, Json] = {}
            for pair in record:
                if (
                    not isinstance(pair, list)
                    or len(pair) != PAIR_SIZE
                    or not isinstance(pair[0], str)
                ):
                    raise ValueError("record is not a mapping or pair sequence")
                result[pair[0]] = pair[1]
            return result
        raise ValueError("record is not a mapping")
    if operation == "to_model":
        if record is None:
            return None
        return record
    if operation == "to_model_list":
        if not isinstance(record, list):
            raise ValueError("record list is not a list")
        return [item for item in record if item is not None]
    raise ValueError("unknown repository_record operation")
