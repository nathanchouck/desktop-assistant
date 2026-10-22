use super::*;

fn record(store: &ConversationStore, title: &str, updated_ms: i64) -> Record {
    Record {
        id: store.new_id(),
        provider: ProviderId::Claude,
        model: "claude-sonnet-5-5".into(),
        session: Some("s-1".into()),
        cwd: "/home/me".into(),
        title: title.into(),
        created_ms: updated_ms,
        updated_ms,
        pinned: false,
        entries: vec![
            ChatEntry::User {
                text: title.into(),
                context: Some("Desktop".into()),
            },
            ChatEntry::Assistant {
                text: "An **answer**".into(),
            },
        ],
    }
}

fn ids(store: &ConversationStore) -> Vec<i64> {
    store.with_summaries(|list| list.iter().map(|s| s.id).collect())
}

#[test]
fn saves_and_loads_a_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let store = ConversationStore::open(&path).unwrap();
    let mut saved = record(&store, "Sort my Downloads", 1_000);
    saved.provider = ProviderId::Chatgpt;
    store.save(&saved).unwrap();

    // A later save replaces the content, keeping when it started.
    saved.entries.push(ChatEntry::Note {
        text: "Stopped".into(),
    });
    saved.updated_ms = 2_000;
    saved.created_ms = 9_999;
    store.save(&saved).unwrap();

    // Read back from disk, as after a restart.
    drop(store);
    let store = ConversationStore::open(&path).unwrap();
    store.load_summaries().unwrap();
    let loaded = store.load(saved.id).unwrap().unwrap();
    assert_eq!(loaded.created_ms, 1_000);
    assert_eq!(loaded.updated_ms, 2_000);
    assert_eq!(
        Record {
            created_ms: 9_999,
            ..loaded
        },
        saved
    );
    store.with_summaries(|list| {
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].provider, ProviderId::Chatgpt);
        assert_eq!(list[0].messages, 2);
        assert!(list[0].search.contains("sort my downloads"));
    });
    assert!(store.new_id() > saved.id, "ids aren't reused");
    assert_eq!(store.load(999).unwrap(), None);
}

#[test]
fn lists_pinned_first_then_most_recent() {
    let store = ConversationStore::in_memory().unwrap();
    let old = record(&store, "old", 1_000);
    let new = record(&store, "new", 3_000);
    let mid = record(&store, "mid", 2_000);
    for r in [&old, &new, &mid] {
        store.save(r).unwrap();
    }
    assert_eq!(ids(&store), [new.id, mid.id, old.id]);
    assert_eq!(store.toggle_pin(old.id).unwrap(), Some(true));
    assert_eq!(ids(&store), [old.id, new.id, mid.id]);
    assert_eq!(
        store.latest(),
        Some(new.id),
        "the last one used, pinned or not"
    );
    assert_eq!(store.counts(), (3, 1));

    // Saving again keeps the pin.
    store.save(&old).unwrap();
    assert!(store.with_summaries(|l| l[0].pinned));
    assert_eq!(store.toggle_pin(old.id).unwrap(), Some(false));
    assert_eq!(store.toggle_pin(42).unwrap(), None);
}

#[test]
fn prunes_the_oldest_unpinned_and_forgets_by_age() {
    let store = ConversationStore::in_memory().unwrap();
    let records: Vec<Record> = (1..=5)
        .map(|i| record(&store, &format!("chat {i}"), i * 1_000))
        .collect();
    for r in &records {
        store.save(r).unwrap();
    }
    store.toggle_pin(records[0].id).unwrap();

    // Keep 2 unpinned: chats 5 and 4 stay, plus pinned chat 1.
    let dropped = store.prune(2, None).unwrap();
    assert_eq!(dropped, [records[1].id, records[2].id]);
    assert_eq!(ids(&store), [records[0].id, records[4].id, records[3].id]);

    // Anything unpinned last used before 4.5 s goes; pinned stays.
    let dropped = store.prune(500, Some(4_500)).unwrap();
    assert_eq!(dropped, [records[3].id]);
    assert_eq!(ids(&store), [records[0].id, records[4].id]);
    assert_eq!(store.load(records[3].id).unwrap(), None);
}

#[test]
fn delete_and_clear() {
    let store = ConversationStore::in_memory().unwrap();
    let a = record(&store, "a", 1);
    let b = record(&store, "b", 2);
    let c = record(&store, "c", 3);
    for r in [&a, &b, &c] {
        store.save(r).unwrap();
    }
    assert!(store.delete(b.id).unwrap());
    assert!(!store.delete(b.id).unwrap());
    store.toggle_pin(c.id).unwrap();
    assert_eq!(store.clear().unwrap(), [a.id]);
    assert_eq!(ids(&store), [c.id], "pinned chats stay");
}
