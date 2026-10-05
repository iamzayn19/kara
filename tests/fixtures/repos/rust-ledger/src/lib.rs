pub mod counter;

/// A transaction amount in cents. Debits are negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub cents: i64,
}

#[derive(Debug, Default)]
pub struct Ledger {
    entries: Vec<Entry>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn credit(&mut self, cents: i64) {
        self.entries.push(Entry { cents });
    }

    pub fn debit(&mut self, cents: i64) {
        self.entries.push(Entry { cents });
    }

    pub fn balance(&self) -> i64 {
        self.entries.iter().map(|e| e.cents).sum()
    }
}
