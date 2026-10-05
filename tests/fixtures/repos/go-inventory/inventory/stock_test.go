package inventory

import "testing"

func TestReserveFailsWithoutSideEffects(t *testing.T) {
	s := NewStore()
	s.Add("widget", 3)
	if err := s.Reserve("widget", 5); err != ErrInsufficient {
		t.Fatalf("expected ErrInsufficient, got %v", err)
	}
	if got := s.Level("widget"); got != 3 {
		t.Fatalf("level changed after failed reservation: %d", got)
	}
}

func TestReserveRejectsNonPositive(t *testing.T) {
	s := NewStore()
	s.Add("widget", 3)
	if err := s.Reserve("widget", -2); err == nil {
		t.Fatal("negative reservation must fail")
	}
	if got := s.Level("widget"); got != 3 {
		t.Fatalf("level changed: %d", got)
	}
}

func TestLowStockThresholdFromEnv(t *testing.T) {
	t.Setenv("INVENTORY_LOW_STOCK", "10")
	if got := LowStockThreshold(); got != 10 {
		t.Fatalf("threshold = %d, want 10", got)
	}
}
