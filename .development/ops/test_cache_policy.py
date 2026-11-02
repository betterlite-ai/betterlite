import unittest

from cache_policy import GIB, capacity


class CacheTests(unittest.TestCase):
    def test_a_full_toy_cache_grows_only_with_space_above_the_reserve(self) -> None:
        self.assertEqual(capacity(100 * GIB, 10 * GIB), 24 * GIB)
        self.assertEqual(capacity(68 * GIB, 10 * GIB), 14 * GIB)
        self.assertEqual(capacity(40 * GIB, 10 * GIB), 10 * GIB)


if __name__ == "__main__":
    unittest.main()
