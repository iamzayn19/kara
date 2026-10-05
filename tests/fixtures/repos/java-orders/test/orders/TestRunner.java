package orders;

public class TestRunner {
    static int failures = 0;

    static void check(String name, boolean ok, String detail) {
        if (ok) {
            System.out.println("PASS " + name);
        } else {
            failures++;
            System.out.println("FAIL " + name + ": " + detail);
        }
    }

    public static void main(String[] args) {
        Order o = new Order();
        o.add("book", 1500, 2);
        o.add("pen", 250, 4);
        check("totalCents multiplies by quantity", o.totalCents() == 4000, "got " + o.totalCents());

        Order big = new Order();
        big.add("lamp", 5000, 1);
        check("free shipping at threshold", Shipping.costCents(big) == 0, "got " + Shipping.costCents(big));

        Order small = new Order();
        small.add("pen", 250, 1);
        check("flat rate below threshold", Shipping.costCents(small) == 499, "got " + Shipping.costCents(small));

        System.out.println(failures == 0 ? "ALL PASSED" : failures + " FAILED");
        System.exit(failures == 0 ? 0 : 1);
    }
}
