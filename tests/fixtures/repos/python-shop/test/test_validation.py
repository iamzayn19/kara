import unittest

from shop.cart import Cart


class ValidationTest(unittest.TestCase):
    def test_rejects_non_positive_quantity(self):
        cart = Cart()
        with self.assertRaises(ValueError):
            cart.add_item("apple", 120, 0)
        with self.assertRaises(ValueError):
            cart.add_item("apple", 120, -2)

    def test_rejects_negative_price(self):
        with self.assertRaises(ValueError):
            Cart().add_item("apple", -1, 1)

    def test_valid_item(self):
        item = Cart().add_item("apple", 120, 2)
        self.assertEqual(item.subtotal(), 240)


if __name__ == "__main__":
    unittest.main()
