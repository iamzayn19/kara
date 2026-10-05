package inventory

import "errors"

var ErrInsufficient = errors.New("insufficient stock")

type Store struct {
	levels map[string]int
}

func NewStore() *Store {
	return &Store{levels: map[string]int{}}
}

func (s *Store) Add(sku string, qty int) {
	s.levels[sku] += qty
}

// Reserve removes qty units of sku. It fails without changing anything when
// there is not enough stock.
func (s *Store) Reserve(sku string, qty int) error {
	s.levels[sku] -= qty
	if s.levels[sku] < 0 {
		return ErrInsufficient
	}
	return nil
}

func (s *Store) Level(sku string) int {
	return s.levels[sku]
}

func (s *Store) LowStock() []string {
	var out []string
	for sku, n := range s.levels {
		if n <= LowStockThreshold() {
			out = append(out, sku)
		}
	}
	return out
}
