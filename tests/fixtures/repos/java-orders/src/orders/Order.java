package orders;

import java.util.ArrayList;
import java.util.List;

public class Order {
    private final List<Line> lines = new ArrayList<>();

    public record Line(String sku, int unitCents, int quantity) {}

    public void add(String sku, int unitCents, int quantity) {
        lines.add(new Line(sku, unitCents, quantity));
    }

    public int totalCents() {
        int total = 0;
        for (Line l : lines) {
            total += l.unitCents();
        }
        return total;
    }

    public List<Line> lines() {
        return lines;
    }
}
