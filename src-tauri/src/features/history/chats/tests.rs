use super::*;
use crate::settings::Settings;
use std::sync::RwLock;
use tokio::sync::mpsc as tokio_mpsc;

fn history() -> (Arc<ChatHistory>, SharedSettings) {
    let settings: SharedSettings = Arc::new(RwLock::new(Settings::default()));
    let store = ConversationStore::in_memory().unwrap();
    (ChatHistory::new(store, Arc::clone(&settings)), settings)
}

fn here() -> Resumable {
    Resumable::new(ProviderId::Claude, std::env::temp_dir(), Some("s-1".into()))
}

fn titles(history: &ChatHistory) -> Vec<String> {
    history
        .store()
        .with_summaries(|l| l.iter().map(|s| s.title.clone()).collect())
}

fn new_chat() -> Chat {
    let (tx, _rx) = tokio_mpsc::unbounded_channel();
    Chat::new(tx)
}

#[test]
fn records_a_conversation_as_it_goes() {
    let (h, _) = history();
    h.begin(1, here(), "what's 2+2?", Some("Desktop"));
    h.flush();
    assert_eq!(titles(&h), ["what's 2+2?"], "saved as soon as you ask");

    h.event(1, &AiEvent::TextDelta { text: "4".into() });
    h.event(1, &AiEvent::Done);
    h.flush();
    let id = h.store().latest().unwrap();
    let saved = h.store().load(id).unwrap().unwrap();
    assert_eq!(saved.session.as_deref(), Some("s-1"));
    assert_eq!(saved.entries.len(), 2);
    assert_eq!(
        h.overview(),
        HistoryOverview {
            conversations: 1,
            pinned: 0
        }
    );
}

#[test]
fn nothing_is_recorded_while_off() {
    let (h, settings) = history();
    settings.write().unwrap().history.enabled = false;
    h.begin(1, here(), "secret", None);
    h.event(1, &AiEvent::Done);
    h.finish();
    h.flush();
    assert!(titles(&h).is_empty());

    // Turned off mid-chat: later saves are dropped too.
    settings.write().unwrap().history.enabled = true;
    h.begin(2, here(), "first", None);
    h.flush();
    settings.write().unwrap().history.enabled = false;
    h.question(2, "second", None);
    h.flush();
    let id = h.store().latest().unwrap();
    assert_eq!(h.store().load(id).unwrap().unwrap().entries.len(), 1);
}

#[tokio::test]
async fn keeps_to_the_limits() {
    let (h, settings) = history();
    settings.write().unwrap().history.max_conversations = 2;
    for (i, title) in ["one", "two", "three"].iter().enumerate() {
        h.begin(i as u32 + 1, here(), title, None);
        h.flush();
    }
    assert_eq!(titles(&h), ["three", "two"]);

    // A lower limit applies when the settings menu asks.
    settings.write().unwrap().history.max_conversations = 1;
    h.tidy().await.unwrap();
    assert_eq!(titles(&h), ["three"]);
}

#[tokio::test]
async fn deleting_the_live_conversation_stops_recording_it() {
    let (h, _) = history();
    h.begin(1, here(), "oops", None);
    // Deleted while its first save may still be queued: it stays deleted.
    let id = h.store().new_id() - 1;
    assert!(h.delete(id).await.unwrap());
    h.event(1, &AiEvent::Done);
    h.finish();
    h.flush();
    assert!(titles(&h).is_empty());
    assert!(!h.delete(id).await.unwrap());
}

#[tokio::test]
async fn clearing_keeps_pinned_chats() {
    let (h, _) = history();
    h.begin(1, here(), "a", None);
    h.begin(2, here(), "b", None);
    h.flush();
    let pinned = h.store().latest().unwrap();
    assert_eq!(h.toggle_pin(pinned).await.unwrap(), Some(true));
    assert_eq!(h.clear().await.unwrap(), 1);
    h.question(2, "still pinned, still recorded", None);
    h.flush();
    assert_eq!(titles(&h), ["b"]);
    assert_eq!(h.store().load(pinned).unwrap().unwrap().entries.len(), 2);

    // A brand-new chat whose first save is still queued goes too.
    h.begin(3, here(), "new", None);
    assert_eq!(h.clear().await.unwrap(), 1);
    h.event(3, &AiEvent::Done);
    h.flush();
    assert_eq!(titles(&h), ["b"]);
}

#[tokio::test]
async fn reopens_a_saved_conversation_where_it_left_off() {
    let (h, _) = history();
    // ChatGPT: whether Claude Code still has a session depends on this
    // machine (see `claude_sessions` for that).
    let thread = Resumable::new(
        ProviderId::Chatgpt,
        std::env::temp_dir(),
        Some("t-1".into()),
    );
    h.begin(1, thread, "hi", None);
    h.event(
        1,
        &AiEvent::Started {
            model: "gpt-6.1-sol".into(),
        },
    );
    h.event(
        1,
        &AiEvent::TextDelta {
            text: "Hello".into(),
        },
    );
    // Closed mid-answer, then reopened at once: the reopened chat has
    // what was saved on closing.
    h.finish();
    let id = h.store().new_id() - 1;

    let chat = new_chat();
    let opened = h.reopen(&chat, id).await.unwrap();
    assert_eq!(opened.provider, ProviderId::Chatgpt);
    assert_eq!(opened.model, "gpt-6.1-sol");
    assert_eq!(opened.entries.len(), 2);
    assert_eq!(chat.provider().await, Some(ProviderId::Chatgpt));
    let resumable = chat.resumable(opened.conversation).await.unwrap();
    assert_eq!(resumable.session().as_deref(), Some("t-1"));
    assert_eq!(resumable.cwd, std::env::temp_dir());

    // A follow-up is recorded into the same saved chat.
    h.question(opened.conversation, "and you?", None);
    h.flush();
    assert_eq!(h.store().load(id).unwrap().unwrap().entries.len(), 3);
    assert_eq!(h.overview().conversations, 1);

    assert!(h.reopen(&chat, 999).await.is_err());
}

#[tokio::test]
async fn a_chat_whose_folder_is_gone_says_so() {
    let (h, _) = history();
    let gone = Resumable::new(
        ProviderId::Claude,
        "/no/such/folder".into(),
        Some("s".into()),
    );
    h.begin(1, gone, "hi", None);
    h.finish();
    h.flush();
    let id = h.store().latest().unwrap();
    let chat = new_chat();
    let opened = h.reopen(&chat, id).await.unwrap();
    assert!(matches!(
        opened.entries.last(),
        Some(ChatEntry::Note { text }) if text.contains("/no/such/folder")
    ));
    let resumable = chat.resumable(opened.conversation).await.unwrap();
    assert_eq!(resumable.session(), None, "Claude starts over");
}
