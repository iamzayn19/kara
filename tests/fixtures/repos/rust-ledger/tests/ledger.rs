use ledger::Ledger;

#[test]
fn credits_and_debits() {
    let mut l = Ledger::new();
    l.credit(1000);
    l.debit(250);
    assert_eq!(l.balance(), 750);
}

#[test]
fn empty_ledger_is_zero() {
    assert_eq!(Ledger::new().balance(), 0);
}
