use super::{MigrationState, append_import};
use crate::queue::SessionQueue;
use std::path::{Path, PathBuf};
use ulos_core::{
    migration::{self as codec, MigrationFormat},
    transfer::CollisionPolicy,
};

#[test]
fn import_is_atomic_bounded_unique_and_no_clobber() {
    let mut queue = SessionQueue::default();
    queue
        .add(
            "https://example.org/existing".into(),
            PathBuf::from("/downloads/a.zip"),
        )
        .unwrap();
    let batch = codec::parse(
        b"https://example.org/a.zip\nhttps://example.org/a.zip\n",
        MigrationFormat::Text,
    )
    .unwrap();
    append_import(&mut queue, &batch, Path::new("/downloads")).unwrap();
    assert_eq!(queue.items.len(), 3);
    assert_ne!(queue.items[1].destination, queue.items[2].destination);
    for item in &queue.items[1..] {
        assert!(item.is_queued());
        assert_eq!(item.options.connections, 4);
        assert_eq!(item.options.collision, CollisionPolicy::Reject);
    }
    let too_many = codec::parse(
        "https://example.org/a\n".repeat(500).as_bytes(),
        MigrationFormat::Text,
    )
    .unwrap();
    assert!(append_import(&mut queue, &too_many, Path::new("/downloads")).is_err());
    assert_eq!(queue.items.len(), 3);
}

#[test]
fn review_tokens_are_single_use_and_reset_invalidates_stale_callbacks() {
    let mut state = MigrationState::default();
    let token = state.begin();
    assert!(state.accepts(token));
    state.finish();
    assert!(!state.accepts(token));
    let next = state.begin();
    assert!(!state.accepts(token));
    assert!(state.accepts(next));
    state.reset();
    assert!(!state.accepts(next));
}
