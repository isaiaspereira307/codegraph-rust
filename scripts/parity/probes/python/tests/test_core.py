import unittest

import pytest

from app.core import Child, worker
from app import util


@pytest.fixture
def child():
    return Child(None)


def test_run(child):
    child.run()
    worker()
    util.helper(3)


class TestChild(unittest.TestCase):
    def setUp(self):
        self.c = Child.build()

    def test_size(self):
        self.assertEqual(self.c.size, 0)
        helper_in_test()

    def helper_in_test(self):
        return worker()


class TestPlain:
    def test_it(self):
        assert worker() is None


@pytest.mark.parametrize("x", [1])
def check_param(x):
    worker()
