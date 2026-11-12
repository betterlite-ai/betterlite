from typing_extensions import override

from betterlite.types.utils.values import StoredValue

INTERNAL_NAMES = ("_values", "_fields_set")


class AttributeAccess:
    _values: dict[str, StoredValue]
    _fields_set: set[str]

    def __getattr__(self, name: str) -> StoredValue:
        if name in INTERNAL_NAMES:
            raise AttributeError(name)
        values = self._values
        if name in values:
            return values[name]
        raise AttributeError(name)

    @override
    def __setattr__(self, name: str, value: StoredValue) -> None:
        if name in INTERNAL_NAMES:
            object.__setattr__(self, name, value)
            return
        self._values[name] = value
        if not name.startswith("_"):
            self._fields_set.add(name)

    @override
    def __delattr__(self, name: str) -> None:
        if name in self._values:
            del self._values[name]
            return
        try:
            super().__delattr__(name)
        except AttributeError:
            return

    def __getitem__(self, key: str) -> StoredValue:
        return self.__getattr__(key)

    def __setitem__(self, key: str, value: StoredValue) -> None:
        self.__setattr__(key, value)

    def __delitem__(self, key: str) -> None:
        self.__delattr__(key)

    def get(self, key: str, default: StoredValue = None) -> StoredValue:
        values = self._values
        if key in values:
            return values[key]
        return default

    def __contains__(self, key: str) -> bool:
        return key in self._values

    @override
    def __eq__(self, other: object) -> bool:  # ast-grep-ignore: no-loose-types
        match other:
            case AttributeAccess():
                return self.__class__ is other.__class__ and self._values == other._values
            case _other:
                return False

    @override
    def __hash__(self) -> int:
        raise TypeError("response objects are not hashable")
