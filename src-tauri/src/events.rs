//! What services tell the window, and the user (docs/design/modules.md,
//! Preparation, One event channel).
//!
//! A service is given an `Emitter` of its own event type and calls it after a
//! change is committed. It never names the event or knows about Tauri: the
//! module that owns an event type names it by implementing `FrontendEvent`,
//! next to the code that emits it, and `lib.rs` connects each emitter to the
//! window with `to_window`. A `Notifier` shows a macOS notification the same
//! way. Tests pass a closure of their own, such as `Arc::new(|_| {})`.

use std::sync::Arc;

use serde::Serialize;

/// An event the window listens for. A trait is a set of methods a type
/// promises to have; this one promises the name the frontend listens for
/// (`src/lib/ipc.ts`). The payload is the type itself, serialized.
pub trait FrontendEvent: Serialize {
    fn name(&self) -> &'static str;
}

/// Where a service sends its committed changes. `E` is the event type, so one
/// alias serves every module; `Arc<dyn Fn…>` lets every task of a service
/// share one closure across threads. The event is lent, not given, because a
/// listener only reads it and some events (a host job's log) are sent often.
pub type Emitter<E> = Arc<dyn Fn(&E) + Send + Sync>;

/// Shows a macOS notification: a title, then a body.
pub type Notifier = Arc<dyn Fn(String, String) + Send + Sync>;

/// When a notification is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notify {
    Always,
    /// Only when no Brainiac window is focused: the window says it too.
    WhenInactive,
}

/// An emitter that sends each event to the window under its own name.
#[cfg(feature = "app")]
pub fn to_window<E: FrontendEvent + 'static>(handle: tauri::AppHandle) -> Emitter<E> {
    Arc::new(move |event: &E| send(&handle, event))
}

/// Send one event to the window. A failure is logged, never returned: a
/// window that missed an event reads the state again when it next asks.
#[cfg(feature = "app")]
pub fn send<E: FrontendEvent>(handle: &tauri::AppHandle, event: &E) {
    // Tauri's `Emitter` trait gives the handle `emit`; `as _` imports the
    // method without its name, which this module's `Emitter` already uses.
    use tauri::Emitter as _;
    if let Err(e) = handle.emit(event.name(), event) {
        tracing::warn!(error = %e, event = event.name(), "failed to emit an event");
    }
}

/// A notifier that shows macOS notifications from this app.
#[cfg(feature = "app")]
pub fn notifier(handle: tauri::AppHandle, when: Notify) -> Notifier {
    Arc::new(move |title: String, body: String| {
        use tauri::Manager as _;
        use tauri_plugin_notification::NotificationExt;
        if when == Notify::WhenInactive
            && handle
                .webview_windows()
                .values()
                .any(|w| w.is_focused().unwrap_or(false))
        {
            return;
        }
        if let Err(e) = handle
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!(error = %e, "could not show a notification");
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;
    use crate::models::{NoteMissingEvent, TaskChangedEvent};
    use crate::notes::KnowledgeEvent;
    use crate::sharing::CodeSharingChanged;

    /// The names every `FrontendEvent` implementation returns, read from the
    /// source: each string literal inside an `impl … FrontendEvent for`
    /// block, up to the block's closing brace at the start of a line.
    fn names_sent(dir: &Path, names: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                names_sent(&path, names);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("events.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (start, _) in text.match_indices("FrontendEvent for ") {
                let block = &text[start..];
                let block = &block[..block.find("\n}\n").expect("an impl block ends")];
                // Every other piece between quotes is a literal.
                names.extend(block.split('"').skip(1).step_by(2).map(str::to_string));
            }
        }
    }

    /// Each event the backend sends has a listener in the window, and each
    /// listener has an event: a name mistyped on either side fails here.
    #[test]
    fn the_window_listens_for_every_event_by_its_name() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sent = BTreeSet::new();
        names_sent(&root.join("src"), &mut sent);
        let ipc = std::fs::read_to_string(root.join("../src/lib/ipc.ts")).unwrap();
        let heard: BTreeSet<String> = ipc
            .split("listen<")
            .skip(1)
            .filter_map(|rest| rest.split_once(">(\"")?.1.split_once('"'))
            .map(|(name, _)| name.to_string())
            .collect();
        assert_eq!(sent.len(), 12, "{sent:?}");
        assert_eq!(sent, heard);
    }

    /// Notes' one enum reaches the window as its variant's own value, under
    /// the variant's name, as the separate events did before.
    #[test]
    fn a_knowledge_event_is_sent_as_its_inner_value() {
        let inner = TaskChangedEvent {
            task_id: "t1".to_string(),
            version: Some(3),
        };
        let event = KnowledgeEvent::TaskChanged(inner.clone());
        assert_eq!(event.name(), "task_changed");
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::to_value(&inner).unwrap()
        );
        let missing = KnowledgeEvent::NoteMissing(NoteMissingEvent {
            note_id: "n1".to_string(),
        });
        assert_eq!(missing.name(), "note_missing");
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            serde_json::json!({ "note_id": "n1" })
        );
    }

    /// Code sharing carries nothing, sent as `null` as `()` was.
    #[test]
    fn code_sharing_changed_is_sent_as_null() {
        assert_eq!(
            serde_json::to_value(CodeSharingChanged).unwrap(),
            serde_json::to_value(()).unwrap()
        );
    }
}
