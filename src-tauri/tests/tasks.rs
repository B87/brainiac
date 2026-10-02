//! Tasks and Today (SPEC.md, section 6): versions, sorting, dates, and
//! links to notes and repositories.

mod common;

use brainiac_lib::models::{
    ErrorCode, SearchKind, SearchRequest, TaskFields, TaskFilter, TaskStatus, UpdateTaskRequest,
};
use brainiac_lib::notes::KnowledgeEvent;
use chrono::NaiveDate;
use common::{note_id_at, Harness};

fn fields(title: &str) -> TaskFields {
    TaskFields {
        title: title.into(),
        description: String::new(),
        status: TaskStatus::Todo,
        planned_date: None,
        due_date: None,
        note_id: None,
        repository_id: None,
        sorted: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_task_is_to_sort_until_it_gets_a_date_or_is_sorted() {
    let h = Harness::new(false).await;
    let t = h
        .tasks
        .create(fields("  Write   the  plan "))
        .await
        .unwrap();
    assert_eq!(t.title, "Write the plan");
    assert!(t.to_sort);
    assert_eq!(t.version, 1);
    let sorted = h
        .tasks
        .update(UpdateTaskRequest {
            task_id: t.id.clone(),
            expected_version: 1,
            fields: TaskFields {
                sorted: true,
                ..fields("Write the plan")
            },
        })
        .await
        .unwrap();
    assert!(!sorted.to_sort);
    assert_eq!(sorted.version, 2);
    let dated = h
        .tasks
        .create(TaskFields {
            due_date: Some("2026-10-03".into()),
            ..fields("Dated")
        })
        .await
        .unwrap();
    assert!(!dated.to_sort);
    let to_sort = h
        .tasks
        .list(TaskFilter {
            to_sort: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(to_sort.is_empty());
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::TaskChanged(c) if c.task_id == t.id)));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_edits_of_one_version_conflict() {
    let h = Harness::new(false).await;
    let t = h.tasks.create(fields("Shared")).await.unwrap();
    let first = UpdateTaskRequest {
        task_id: t.id.clone(),
        expected_version: t.version,
        fields: fields("From the UI"),
    };
    let second = UpdateTaskRequest {
        task_id: t.id.clone(),
        expected_version: t.version,
        fields: fields("From an agent"),
    };
    h.tasks.update(first).await.unwrap();
    let err = h.tasks.update(second).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(h.tasks.get(&t.id).await.unwrap().title, "From the UI");
    let err = h.tasks.delete(&t.id, t.version).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    h.tasks.delete(&t.id, t.version + 1).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn completing_records_when_and_reopening_clears_it() {
    let h = Harness::new(false).await;
    let t = h.tasks.create(fields("Ship")).await.unwrap();
    let done = h
        .tasks
        .update(UpdateTaskRequest {
            task_id: t.id.clone(),
            expected_version: t.version,
            fields: TaskFields {
                status: TaskStatus::Done,
                ..fields("Ship")
            },
        })
        .await
        .unwrap();
    assert!(done.completed_at.is_some());
    let reopened = h
        .tasks
        .update(UpdateTaskRequest {
            task_id: t.id.clone(),
            expected_version: done.version,
            fields: fields("Ship"),
        })
        .await
        .unwrap();
    assert_eq!(reopened.completed_at, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_tasks_are_refused() {
    let h = Harness::new(false).await;
    for bad in [
        fields("   "),
        TaskFields {
            due_date: Some("2026-02-30".into()),
            ..fields("x")
        },
        TaskFields {
            planned_date: Some("3 Oct".into()),
            ..fields("x")
        },
        TaskFields {
            title: "x".repeat(301),
            ..fields("x")
        },
        TaskFields {
            note_id: Some("no-such-note".into()),
            ..fields("x")
        },
        TaskFields {
            repository_id: Some("no-such-repo".into()),
            ..fields("x")
        },
    ] {
        let err = h.tasks.create(bad).await.unwrap_err();
        assert!(
            matches!(err.code, ErrorCode::Validation | ErrorCode::NotFound),
            "{err:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn today_lists_overdue_due_and_planned_work_by_local_date() {
    let h = Harness::new(false).await;
    let make = |title: &str, planned: Option<&str>, due: Option<&str>| TaskFields {
        planned_date: planned.map(str::to_string),
        due_date: due.map(str::to_string),
        ..fields(title)
    };
    h.tasks
        .create(make("Overdue", None, Some("2026-10-01")))
        .await
        .unwrap();
    h.tasks
        .create(make("Due today", None, Some("2026-10-02")))
        .await
        .unwrap();
    h.tasks
        .create(make("Planned today", Some("2026-10-02"), None))
        .await
        .unwrap();
    h.tasks
        .create(make("Planned yesterday", Some("2026-10-01"), None))
        .await
        .unwrap();
    h.tasks
        .create(make("Tomorrow", Some("2026-10-03"), Some("2026-10-09")))
        .await
        .unwrap();
    h.tasks.create(make("Unsorted", None, None)).await.unwrap();

    let today = h
        .tasks
        .today_on(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap())
        .await
        .unwrap();
    assert_eq!(today.date, "2026-10-02");
    let titles: Vec<&str> = today.open.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Overdue", "Due today", "Planned yesterday", "Planned today"]
    );
    assert_eq!(today.to_sort.len(), 1);

    // After midnight the same tasks move: today follows the date.
    let tomorrow = h
        .tasks
        .today_on(NaiveDate::from_ymd_opt(2026, 10, 3).unwrap())
        .await
        .unwrap();
    assert!(tomorrow.open.iter().any(|t| t.title == "Tomorrow"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_linked_to_a_note_and_repository_shows_in_their_context() {
    let h = Harness::new(false).await;
    h.write("Release.md", "# Release\nsteps\n");
    h.scan().await;
    let note = note_id_at(&h, "Release.md").await;
    let repo = h.register_repository("web").await;
    let t = h
        .tasks
        .create(TaskFields {
            note_id: Some(note.clone()),
            repository_id: Some(repo.clone()),
            planned_date: Some("2026-10-02".into()),
            ..fields("Tag the release")
        })
        .await
        .unwrap();
    assert_eq!(t.note.as_ref().unwrap().title, "Release");
    // The note now carries its ID, because a task depends on it.
    assert!(h
        .read("Release.md")
        .starts_with(&format!("---\nbrainiac_id: {note}\n---\n")));
    assert_eq!(h.notes.context(&note).await.unwrap().tasks.len(), 1);
    assert_eq!(
        h.notes.repository_notes(&repo).await.unwrap().tasks.len(),
        1
    );
    // A fixed date: reading the clock twice could straddle midnight.
    let today = h
        .tasks
        .today_on(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap())
        .await
        .unwrap();
    assert_eq!(today.repository_ids, vec![repo]);

    let found = brainiac_lib::index::search(
        &h.notes,
        SearchRequest {
            query: "relea".into(),
            kinds: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(found.tasks.hits.len(), 1);
    assert_eq!(found.notes.hits.len(), 1);
    let found = brainiac_lib::index::search(
        &h.notes,
        SearchRequest {
            query: "\"tag the\"".into(),
            kinds: Some(vec![SearchKind::Task]),
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(found.tasks.total, 1);
    assert!(found.notes.hits.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_a_repository_announces_the_tasks_and_notes_it_moved() {
    let h = Harness::new(false).await;
    h.write("Release.md", "# Release\n");
    h.scan().await;
    let note = note_id_at(&h, "Release.md").await;
    let old = h.register_repository("web").await;
    let new = h.register_repository("web-clone").await;
    h.notes.link_repository(&note, &old).await.unwrap();
    let t = h
        .tasks
        .create(TaskFields {
            note_id: Some(note.clone()),
            repository_id: Some(old.clone()),
            ..fields("Tag the release")
        })
        .await
        .unwrap();
    h.events.lock().unwrap().clear();

    h.notes.reconnect_repository(&old, &new).await.unwrap();
    let moved = h.tasks.get(&t.id).await.unwrap();
    assert_eq!(moved.repository_id.as_deref(), Some(new.as_str()));
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::TaskChanged(c)
        if c.task_id == t.id && c.version == Some(moved.version))));
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::NoteChanged(c) if c.note_id == note)));

    // A view that took the announced version can edit the task.
    h.tasks
        .update(UpdateTaskRequest {
            task_id: t.id.clone(),
            expected_version: moved.version,
            fields: fields("Tag the release today"),
        })
        .await
        .unwrap();
}
