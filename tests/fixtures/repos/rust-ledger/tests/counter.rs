use ledger::counter::RequestCounter;
use std::sync::Arc;

#[test]
fn concurrent_records_are_not_lost() {
    let c = Arc::new(RequestCounter::new());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let c = c.clone();
            std::thread::spawn(move || {
                for _ in 0..5_000 {
                    c.record();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(c.get(), 40_000);
}
