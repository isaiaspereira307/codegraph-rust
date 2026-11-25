import unittest
from shapes import Shape


class Circle(Shape):
    def area(self):
        return 3


class CircleCase(unittest.TestCase):
    def test_area(self):
        self.assertEqual(Circle().area(), 3)
