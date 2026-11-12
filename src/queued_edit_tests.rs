use super::*;

fn queued() -> Thread {
    let mut t = Thread::new(1);
    t.status = Status::Running;
    t.model_provider = "cpa".into();
    t.native.follow_up = true;
    t.draft.replace_text("Queued original");
    t.send_during_run();
    t.draft.replace_text("Ordinary draft");
    t
}

#[test]
fn queued_edit_is_isolated_cancel_discards_and_save_updates_in_place() {
    let mut t = queued();
    let id = t.native.queue[0].id;
    t.edit_queued(id);
    assert_eq!(t.native.queue.len(), 1);
    assert_eq!(t.draft.text, "Queued original");
    t.draft.replace_text("Discard this edit");
    t.cancel_queued_edit();
    assert_eq!(t.draft.text, "Ordinary draft");
    assert_eq!(t.native.queue[0].prompt.text, "Queued original");
    t.edit_queued(id);
    t.draft.replace_text("Saved edit");
    t.save_queued_edit();
    assert!(t.queued_edit.is_none());
    assert_eq!(t.draft.text, "Ordinary draft");
    assert_eq!(t.native.queue[0].id, id);
    assert_eq!(t.native.queue[0].prompt.text, "Saved edit");
}

#[test]
fn queue_removal_or_start_recovers_changed_edit_only_into_an_empty_draft() {
    for ordinary_empty in [false, true] {
        for started in [false, true] {
            let mut t = queued();
            if ordinary_empty {
                t.draft.replace_text("");
            }
            let id = t.native.queue[0].id;
            t.edit_queued(id);
            t.draft.replace_text("Unsaved edit");
            if started {
                t.native.queue.pop_front();
                t.reconcile_queued_edit();
            } else {
                t.cancel_queued(id);
            }
            assert!(t.queued_edit.is_none());
            assert_eq!(
                t.draft.text,
                if ordinary_empty {
                    "Unsaved edit"
                } else {
                    "Ordinary draft"
                }
            );
            assert!(t.error.as_ref().unwrap().contains("no longer queued"));
        }
    }
}

#[test]
fn queued_edit_checks_generation_identity_and_concurrent_changes() {
    let mut t = queued();
    let id = t.native.queue[0].id;
    let revision = queue_revision(&t.native.queue[0].prompt);
    let generation = t.request_generation.clone();
    let changed = Prompt::new("Other client's edit".into(), vec![]);
    t.update_queued(id, &generation, &revision, changed)
        .unwrap();
    let stale = Prompt::new("Stale edit".into(), vec![]);
    assert!(
        t.update_queued(id, &generation, &revision, stale.clone())
            .is_err()
    );
    let revision = queue_revision(&t.native.queue[0].prompt);
    assert!(
        t.update_queued(id, "old runtime", &revision, stale.clone())
            .is_err()
    );
    t.native.queue.clear();
    assert!(t.update_queued(id, &generation, &revision, stale).is_err());
}

#[test]
fn queued_save_receipt_cannot_close_a_replacement_edit() {
    let mut t = queued();
    let id = t.native.queue[0].id;
    t.edit_queued(id);
    let identity = t.queued_edit.as_ref().unwrap().identity;
    let command = uuid::Uuid::new_v4();
    t.queued_save = Some((command, identity));
    t.cancel_queued_edit();
    t.edit_queued(id);
    t.draft.replace_text("New edit");
    t.finish_queued_save(command, true);
    assert_eq!(t.draft.text, "New edit");
    assert!(t.queued_edit.is_some());
}

#[test]
fn queued_edit_keeps_rich_marks_and_edits_attachments_without_touching_the_draft() {
    let mut t = queued();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let file = attachments::reference_file(&path).unwrap();
    t.native.queue[0].prompt = Prompt::new("Queued original".into(), vec![file.clone()]);
    let ordinary = crate::composer_rich::Snapshot::capture(&t.draft, &t.draft_images);
    let id = t.native.queue[0].id;
    t.edit_queued(id);
    t.draft_images.clear();
    t.draft.set_selection(0, 6);
    t.draft.toggle_mark(rune::Mark::Bold);
    t.save_queued_edit();
    assert!(t.native.queue[0].prompt.images.is_empty());
    assert_eq!(t.native.queue[0].prompt.text, "**Queued** original");
    assert_eq!(
        crate::composer_rich::Snapshot::capture(&t.draft, &t.draft_images),
        ordinary
    );
    assert_eq!(
        t.native.queue[0]
            .prompt
            .draft
            .as_ref()
            .unwrap()
            .editor()
            .marks()
            .len(),
        1
    );
}

#[test]
fn a_saving_edit_waits_for_its_receipt_before_recovering_a_removed_item() {
    let mut t = queued();
    t.draft.replace_text("");
    let id = t.native.queue[0].id;
    t.edit_queued(id);
    t.draft.replace_text("Unsaved");
    let command = uuid::Uuid::new_v4();
    let identity = t.queued_edit.as_ref().unwrap().identity;
    t.queued_save = Some((command, identity));
    t.native.queue.clear();
    t.reconcile_queued_edit();
    assert!(t.queued_edit.is_some());
    t.finish_queued_save(command, false);
    assert!(t.queued_edit.is_none());
    assert_eq!(t.draft.text, "Unsaved");
}

#[test]
fn older_servers_block_transactional_edits_without_removing_the_item() {
    let mut t = queued();
    t.queue_edit_supported = false;
    t.edit_queued(t.native.queue[0].id);
    assert!(t.queued_edit.is_none());
    assert_eq!(t.native.queue.len(), 1);
    assert!(t.error.as_ref().unwrap().contains("Update required"));
}
