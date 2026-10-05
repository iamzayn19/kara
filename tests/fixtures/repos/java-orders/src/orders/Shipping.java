package orders;

public final class Shipping {
    public static final int FREE_SHIPPING_THRESHOLD_CENTS = 5000;
    public static final int FLAT_RATE_CENTS = 499;

    private Shipping() {}

    /** Orders at or above the threshold ship free. */
    public static int costCents(Order order) {
        return order.totalCents() > FREE_SHIPPING_THRESHOLD_CENTS ? 0 : FLAT_RATE_CENTS;
    }
}
