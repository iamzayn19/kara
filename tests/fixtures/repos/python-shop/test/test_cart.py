import unittest

from shop.cart import Cart


class CartTest(unittest.TestCase):
    def setUp(self):
        self.cart = Cart()
        self.cart.add_item("apple", 120, 3)
        self.cart.add_item("pear", 200, 2)

    def test_total(self):
        self.assertEqual(self.cart.total(), 760)

    def test_discount_reduces_total(self):
        self.assertEqual(self.cart.apply_discount(25), 570)

    def test_zero_discount(self):
        self.assertEqual(self.cart.apply_discount(0), 760)

    def test_invalid_discount(self):
        with self.assertRaises(ValueError):
            self.cart.apply_discount(150)


if __name__ == "__main__":
    unittest.main()
