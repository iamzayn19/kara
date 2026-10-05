package inventory

import (
	"os"
	"strconv"
)

// LowStockThreshold reads INVENTORY_LOW_STOCK (default 5).
func LowStockThreshold() int {
	v, err := strconv.Atoi(os.Getenv("INVENTORY_LOW_STOCK_THRESHOLD"))
	if err != nil {
		return 5
	}
	return v
}
