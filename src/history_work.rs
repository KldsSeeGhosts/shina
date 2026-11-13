//! KSG `threadHistoryPaging.ts:176-283`: deferred completed work, retained by first item.
use crate::history::Row;
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

pub const INLINE_WORK_ITEMS: usize = 150;

fn work_start(row: &Row, starts: &[usize]) -> Option<usize> {
    let message = row.item["anchor"]["message"].as_u64()? as usize;
    starts
        .iter()
        .rev()
        .find(|&&start| start <= message)
        .copied()
}

fn deferrable(row: &Row) -> bool {
    if row.item["done"] != true {
        return false;
    }
    if row.field == "thoughts" {
        return true;
    }
    if row.field != "tools" || row.item["failed"] == true {
        return false;
    }
    // Shina keeps viewed-image output as attachments rather than KSG's viewedImagePath.
    if row.item["images"]
        .as_array()
        .is_some_and(|images| !images.is_empty())
    {
        return false;
    }
    let kind = row.item["kind"].as_str().unwrap_or("");
    let name = row.item["name"].as_str().unwrap_or("");
    if matches!(kind, "plan" | "html" | "subagent")
        || name == "plan"
        || name.contains("delegate_task")
        || name.ends_with("html_render")
    {
        return false;
    }
    let payload = row.item.to_string();
    !["htmlRender", "t3McpApp", "viewedImagePath"]
        .iter()
        .any(|key| payload.contains(key))
}

fn groups<'a>(rows: &[&'a Row], completed_only: bool) -> Vec<Vec<&'a Row>> {
    let mut starts: Vec<_> = rows
        .iter()
        .filter(|row| row.field == "work")
        .filter_map(|row| row.item["start"].as_u64().map(|start| start as usize))
        .collect();
    starts.sort_unstable();
    starts.dedup();
    let completed: HashSet<_> = rows
        .iter()
        .filter(|row| {
            row.field == "work" && row.item["outcome"] == "Completed" && row.item["stopped"] != true
        })
        .filter_map(|row| row.item["start"].as_u64().map(|start| start as usize))
        .collect();
    let mut groups: HashMap<usize, Vec<&Row>> = HashMap::new();
    for row in rows {
        if deferrable(row)
            && let Some(start) = work_start(row, &starts)
            && (!completed_only || completed.contains(&start))
        {
            groups.entry(start).or_default().push(*row);
        }
    }
    groups.into_values().collect()
}

pub fn deferred<'a>(rows: &[&'a Row]) -> Vec<Cow<'a, Row>> {
    let mut omitted = HashSet::new();
    let mut anchors = HashMap::new();
    for group in groups(rows, true)
        .into_iter()
        .filter(|group| group.len() > INLINE_WORK_ITEMS)
    {
        let first = group[0];
        let last = group.last().unwrap();
        for row in &group[1..] {
            omitted.insert(row.source_item_id.clone());
        }
        let mut anchor = first.clone();
        let starts: Vec<_> = rows
            .iter()
            .filter(|row| row.field == "work")
            .filter_map(|row| row.item["start"].as_u64().map(|start| start as usize))
            .collect();
        anchor.item["deferredWork"] = json!({
            "itemCount":group.len(), "lastOrdinal":last.position,
            "start":work_start(first, &starts),
            "cursor":json!({"v":2,"seq":0,"st":last.source_thread_id,
                "si":last.source_item_id,"p":last.position,
                "sortKey":crate::history::timeline_sort_key(last, 0),"inclusive":true}).to_string()
        });
        anchors.insert(first.source_item_id.clone(), anchor);
    }
    rows.iter()
        .copied()
        .filter(|row| !omitted.contains(&row.source_item_id))
        .map(|row| {
            anchors
                .remove(&row.source_item_id)
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed(row))
        })
        .collect()
}

pub fn page(
    rows: &[&Row],
    anchor: &str,
    cursor: &str,
    sequence: u64,
    budget: usize,
) -> Result<Value, String> {
    if cursor.len() > 4096 {
        return Err("Invalid work history cursor.".into());
    }
    let group = groups(rows, false)
        .into_iter()
        .find(|group| group.iter().any(|row| row.source_item_id == anchor))
        .ok_or("Work history changed. Retry loading it.")?;
    let start = group
        .iter()
        .position(|row| row.source_item_id == anchor)
        .unwrap();
    let group = &group[start..];
    let cursor: Value = serde_json::from_str(cursor).map_err(|_| "Invalid work history cursor.")?;
    if cursor["v"] != 2
        || !cursor["seq"].is_u64()
        || !cursor["p"].is_u64()
        || !cursor["si"].is_string()
        || cursor["st"] != group[0].source_thread_id
        || cursor
            .get("inclusive")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err("Invalid work history cursor.".into());
    }
    let end = if let Some(last) = group
        .iter()
        .position(|row| row.source_item_id == cursor["si"])
    {
        last + usize::from(cursor["inclusive"] == true)
    } else {
        let key: (usize, u8, u64, usize, usize) = serde_json::from_value(cursor["sortKey"].clone())
            .map_err(|_| "Work history changed. Retry loading it.")?;
        group
            .iter()
            .position(|row| {
                let row_key = crate::history::timeline_sort_key(row, 0);
                if cursor["inclusive"] == true {
                    row_key > key
                } else {
                    row_key >= key
                }
            })
            .unwrap_or(group.len())
    };
    let mut selected = Vec::new();
    let mut bytes = 0;
    for row in group[..end].iter().rev() {
        let size = serde_json::to_vec(row)
            .map_err(|error| error.to_string())?
            .len();
        if !selected.is_empty()
            && (selected.len() >= crate::history::PAGE_ITEMS || bytes + size > budget)
        {
            break;
        }
        selected.push(*row);
        bytes += size;
    }
    selected.reverse();
    let first = selected.first().ok_or("Work history did not advance.")?;
    let more = first.source_item_id != group[0].source_item_id;
    let next = more.then(|| {
        json!({"v":2,"seq":sequence,"st":first.source_thread_id,
        "si":first.source_item_id,"p":first.position,
        "sortKey":crate::history::timeline_sort_key(first, 0)})
        .to_string()
    });
    Ok(json!({"items":selected,"nextCursor":next,"hasMoreHistory":more}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defer(rows: Vec<Row>) -> Vec<Row> {
        deferred(&rows.iter().collect::<Vec<_>>())
            .into_iter()
            .map(Cow::into_owned)
            .collect()
    }
    fn rows(count: usize) -> Vec<Row> {
        let mut rows = vec![Row {
            source_thread_id: 1,
            source_item_id: "work".into(),
            field: "work".into(),
            ordinal: 0,
            position: 0,
            item: json!({"start":1,"outcome":"Completed","stopped":false}),
        }];
        rows.extend((0..count).map(|i| Row {
            source_thread_id: 1,
            source_item_id: format!("tool-{i}"),
            field: "tools".into(),
            ordinal: i,
            position: i + 1,
            item: json!({"id":format!("tool-{i}"),"done":true,
                "anchor":{"message":1},"output":"x".repeat(2048)}),
        }));
        rows
    }
    #[test]
    fn above_150_completed_items_defer_and_500_load_in_bounded_work_pages() {
        assert_eq!(defer(rows(150)).len(), 151);
        let raw = rows(500);
        let deferred = defer(raw.clone());
        assert_eq!(deferred.len(), 2);
        assert_eq!(deferred[1].item["deferredWork"]["itemCount"], 500);
        let mut cursor = deferred[1].item["deferredWork"]["cursor"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut loaded = HashSet::new();
        loop {
            let page = page(&raw.iter().collect::<Vec<_>>(), "tool-0", &cursor, 1, 65536).unwrap();
            for row in page["items"].as_array().unwrap() {
                assert!(loaded.insert(row["sourceItemId"].as_str().unwrap().to_owned()));
            }
            let Some(next) = page["nextCursor"].as_str() else {
                break;
            };
            assert_ne!(next, cursor);
            cursor = next.into();
        }
        assert_eq!(loaded.len(), 500);
    }
    #[test]
    fn unknown_live_failed_stopped_and_persistent_work_do_not_defer() {
        for outcome in [Value::Null, json!("Running"), json!("Failed")] {
            let mut raw = rows(151);
            raw[0].item["outcome"] = outcome;
            assert_eq!(defer(raw).len(), 152);
        }
        let mut raw = rows(151);
        raw[0].item["stopped"] = json!(true);
        assert_eq!(defer(raw).len(), 152);
        for key in ["htmlRender", "t3McpApp", "viewedImagePath"] {
            let mut raw = rows(151);
            raw[1].item[key] = json!(true);
            assert_eq!(defer(raw).len(), 152);
        }
        let mut raw = rows(151);
        raw[1].item["images"] = json!([{"id":"viewed.png"}]);
        assert_eq!(defer(raw).len(), 152);
    }
    #[test]
    fn reasoning_and_all_tool_work_share_the_cutoff_but_conversation_and_persistent_output_stay() {
        let mut raw = rows(151);
        raw[1].field = "thoughts".into();
        raw[1].item["text"] = json!("Reasoning");
        for (row, kind) in raw[2..].iter_mut().zip(
            [
                "command",
                "file_change",
                "file_search",
                "web_search",
                "other",
            ]
            .into_iter()
            .cycle(),
        ) {
            row.item["kind"] = json!(kind);
        }
        let mut plan = raw[2].clone();
        plan.source_item_id = "plan".into();
        plan.item["kind"] = json!("plan");
        raw.push(plan);
        let mut persistent = raw[2].clone();
        persistent.source_item_id = "persistent".into();
        persistent.item["structured"] = json!({"htmlRender":{}});
        raw.push(persistent);
        let mut compaction = raw[2].clone();
        compaction.source_item_id = "compaction".into();
        compaction.field = "compactions".into();
        raw.push(compaction);
        let deferred = defer(raw);
        assert_eq!(
            deferred
                .iter()
                .map(|row| row.source_item_id.as_str())
                .collect::<Vec<_>>(),
            ["work", "tool-0", "plan", "persistent", "compaction"]
        );
        assert_eq!(deferred[1].item["deferredWork"]["itemCount"], 151);
    }

    #[test]
    fn separate_turns_are_not_combined_and_work_pages_survive_deleted_cursor_rows() {
        let mut raw = rows(151);
        raw[0].item["start"] = json!(1);
        let mut second = raw[0].clone();
        second.source_item_id = "second".into();
        second.item["start"] = json!(2);
        raw.push(second);
        for row in raw
            .iter_mut()
            .filter(|row| row.ordinal >= 75 && row.field == "tools")
        {
            row.item["anchor"]["message"] = json!(2);
        }
        assert_eq!(defer(raw).len(), 153);

        let mut raw = rows(500);
        let deferred = defer(raw.clone());
        let cursor = deferred[1].item["deferredWork"]["cursor"].as_str().unwrap();
        raw.pop();
        let result = page(&raw.iter().collect::<Vec<_>>(), "tool-0", cursor, 1, 65536).unwrap();
        assert_eq!(
            result["items"].as_array().unwrap().last().unwrap()["sourceItemId"],
            "tool-498"
        );
        assert!(page(&raw.iter().collect::<Vec<_>>(), "tool-0", "{}", 1, 65536).is_err());
    }
}
