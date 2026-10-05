import unittest

from shop.cart import Cart
from shop.report import summary


class ReportTest(unittest.TestCase):
    def test_summary(self):
        cart = Cart()
        cart.add_item("apple", 120, 3)
        self.assertEqual(summary(cart), "apple x3: $3.60\nTotal: $3.60")


if __name__ == "__main__":
    unittest.main()
