use super::*;

#[test]
fn query_tokens_are_distinct() {
    let a = NEXT_QUERY.fetch_add(1, Ordering::Relaxed);
    let b = NEXT_QUERY.fetch_add(1, Ordering::Relaxed);
    assert_ne!(a, b);
}
