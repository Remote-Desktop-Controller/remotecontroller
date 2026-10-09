pub fn remaining_budget(budget_ms: u64, elapsed: std::time::Duration) -> u64 {
    budget_ms
        .min(120_000)
        .saturating_sub(elapsed.as_micros().div_ceil(1000).min(u64::MAX as u128) as u64)
}
#[cfg(test)]
mod budget_tests {
    #[test]
    fn remote_budget_subtracts_full_roundtrip_and_never_grows_on_delayed_execution() {
        assert_eq!(
            super::remaining_budget(1000, std::time::Duration::from_millis(700)),
            300
        );
        assert_eq!(
            super::remaining_budget(1000, std::time::Duration::from_millis(1001)),
            0
        );
        assert_eq!(
            super::remaining_budget(u64::MAX, std::time::Duration::ZERO),
            120000
        );
    }
}
