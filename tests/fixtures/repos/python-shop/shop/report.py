"""Order summary report."""

from shop.format import money


def summary(cart):
    lines = [f"{item.sku} x{item.quantity}: {money(item.subtotal())}" for item in cart.items]
    lines.append(f"Total: {money(cart.total())}")
    return "\n".join(lines)
