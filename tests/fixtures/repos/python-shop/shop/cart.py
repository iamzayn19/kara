"""Shopping cart."""


class LineItem:
    def __init__(self, sku, price_cents, quantity):
        self.sku = sku
        self.price_cents = price_cents
        self.quantity = quantity

    def subtotal(self):
        return self.price_cents * self.quantity


class Cart:
    def __init__(self):
        self.items = []

    def add_item(self, sku, price_cents, quantity=1):
        item = LineItem(sku, price_cents, quantity)
        self.items.append(item)
        return item

    def total(self):
        return sum(item.subtotal() for item in self.items)

    def apply_discount(self, percent):
        """Return the total after a percentage discount, rounded to cents."""
        if percent < 0 or percent > 100:
            raise ValueError("percent must be between 0 and 100")
        return round(self.total() * percent / 100)
