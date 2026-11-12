//! Row projections ported from KSG ProjectionStore.apply.
//! A scope is one thread, not the workspace. Payload/schema versions are explicit.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Rows = BTreeMap<String, Value>;

fn scopes(value: &Value) -> BTreeMap<String, Rows> {
    let mut result = BTreeMap::new();
    let mut root = value.clone();
    let partial = root.as_object_mut().unwrap().remove("_partial") == Some(json!(true));
    for field in ["threads", "threadViews"] {
        if let Some(items) = root.as_object_mut().unwrap().remove(field) {
            let items = items.as_array().unwrap();
            if !partial {
                root[field] = json!(items.iter().map(|i| i["id"].clone()).collect::<Vec<_>>());
            }
            for item in items {
                let scope = format!("{field}/{}", item["id"]);
                let mut body = item.clone();
                let identities = body
                    .as_object_mut()
                    .unwrap()
                    .remove("_history")
                    .unwrap_or(json!({}));
                let mut rows = Rows::new();
                if field == "threadViews" || field == "threads" && item.get("messages").is_some() {
                    // Auxiliary arrays such as skills must not inflate the small
                    // header rewritten for a history sequence. Preserve their wire
                    // location while storing their entries beside history rows.
                    let fields = body
                        .as_object()
                        .unwrap()
                        .iter()
                        .filter(|(_, value)| value.is_array())
                        .map(|(field, _)| field.clone())
                        .collect::<Vec<_>>();
                    for field in &fields {
                        let field = field.as_str();
                        if let Some(items) = body[field].as_array().cloned() {
                            body[field] = json!([]);
                            for (ordinal, item) in items.into_iter().enumerate() {
                                let key = format!(
                                    "{field}/{}",
                                    identities[field][ordinal]["id"]
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| ordinal.to_string())
                                );
                                rows.insert(
                                    key,
                                    json!({"ordinal":ordinal,"item":item,"identity":identities[field][ordinal]}),
                                );
                            }
                        }
                    }
                }
                rows.insert("$".into(), body);
                result.insert(scope, rows);
            }
        }
    }
    for field in ["runtimeInputs", "pendingMessages"] {
        if let Some(inputs) = root.as_object_mut().unwrap().remove(field) {
            if let Some(inputs) = inputs.as_object() {
                if !partial {
                    root[field] = json!(inputs.keys().collect::<Vec<_>>());
                }
                for (id, value) in inputs {
                    result.insert(
                        format!("{field}/{id}"),
                        BTreeMap::from([("$".into(), value.clone())]),
                    );
                }
            }
        }
    }
    if !partial || root.as_object().is_some_and(|r| !r.is_empty()) {
        result.insert(
            "$".into(),
            root.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
    }
    result
}

pub fn read(connection: &Connection, document: &str) -> Result<Option<Value>, String> {
    let mut statement = connection
        .prepare(
            "SELECT key,payload,schema FROM projection_records WHERE document=?1 AND scope='$'",
        )
        .map_err(|e| e.to_string())?;
    let records = statement
        .query_map([document], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if records.is_empty() {
        return Ok(None);
    }
    let mut root = json!({});
    for (key, text, schema) in records {
        if schema != 1 {
            return Err("Projection row requires a newer Shina binary.".into());
        }
        root[&key] = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    }
    for field in ["threads", "threadViews", "runtimeInputs", "pendingMessages"] {
        let Some(ids) = root[field].as_array().cloned() else {
            continue;
        };
        let mut array = Vec::new();
        let mut inputs = json!({});
        for id in ids {
            let id = id
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| id.to_string());
            let mut stmt = connection.prepare(
                "SELECT key,payload,schema FROM projection_records WHERE document=?1 AND scope=?2 ORDER BY key"
            ).map_err(|e| e.to_string())?;
            let records = stmt
                .query_map(params![document, format!("{field}/{id}")], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u32>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let mut body = Value::Null;
            let mut history = json!({});
            let mut entries = BTreeMap::new();
            for (key, text, schema) in records {
                if schema != 1 {
                    return Err("Projection row requires a newer Shina binary.".into());
                }
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                if key == "$" {
                    body = value;
                    continue;
                }
                let (field, _) = key.split_once('/').ok_or("Invalid projection row key.")?;
                let ordinal = value["ordinal"]
                    .as_u64()
                    .ok_or("Invalid projection ordinal.")?;
                entries.insert((field.to_owned(), ordinal), value);
            }
            for ((field, _), value) in entries {
                if !body[&field].is_array() {
                    return Err("Invalid projection item array.".into());
                }
                body[&field]
                    .as_array_mut()
                    .unwrap()
                    .push(value["item"].clone());
                if crate::protocol::HISTORY_FIELDS.contains(&field.as_str()) {
                    if !history[&field].is_array() {
                        history[&field] = json!([]);
                    }
                    history[&field]
                        .as_array_mut()
                        .unwrap()
                        .push(value["identity"].clone());
                }
            }
            if body.is_null() {
                return Err("Missing projection scope.".into());
            }
            if history.as_object().is_some_and(|h| !h.is_empty()) {
                body["_history"] = history;
            }
            if matches!(field, "runtimeInputs" | "pendingMessages") {
                inputs[&id] = body;
            } else {
                array.push(body);
            }
        }
        root[field] = if matches!(field, "runtimeInputs" | "pendingMessages") {
            inputs
        } else {
            json!(array)
        };
    }
    Ok(Some(root))
}

pub fn write(connection: &Connection, document: &str, value: &Value) -> Result<(), String> {
    let partial = value["_partial"] == true;
    let scopes = scopes(value);
    if !partial {
        let mut statement = connection
            .prepare("SELECT DISTINCT scope FROM projection_records WHERE document=?1")
            .map_err(|e| e.to_string())?;
        let old = statement
            .query_map([document], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for scope in old {
            if !scopes.contains_key(&scope) {
                connection
                    .execute(
                        "DELETE FROM projection_records WHERE document=?1 AND scope=?2",
                        params![document, scope],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    for (scope, rows) in scopes {
        if partial && scope == "$" {
            for (key, value) in rows {
                let old:Option<String>=connection.query_row("SELECT payload FROM projection_records WHERE document=?1 AND scope='$' AND key=?2",params![document,key],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
                if old
                    .map(|text| serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()))
                    .transpose()?
                    .as_ref()
                    == Some(&value)
                {
                    continue;
                }
                connection.execute("INSERT INTO projection_records(document,scope,key,schema,payload) VALUES(?1,'$',?2,1,?3) ON CONFLICT(document,scope,key) DO UPDATE SET schema=excluded.schema,payload=excluded.payload",
                    params![document,key,crate::database::encode(&value)]).map_err(|e|e.to_string())?;
            }
            continue;
        }
        let mut statement = connection
            .prepare("SELECT key,payload FROM projection_records WHERE document=?1 AND scope=?2")
            .map_err(|e| e.to_string())?;
        let old = statement
            .query_map(params![document, scope], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let old: BTreeMap<String, Value> = old
            .into_iter()
            .map(|(k, v)| Ok((k, serde_json::from_str(&v).map_err(|e| e.to_string())?)))
            .collect::<Result<_, String>>()?;
        for key in old
            .keys()
            .filter(|key| !(partial && scope == "$") && !rows.contains_key(*key))
        {
            connection
                .execute(
                    "DELETE FROM projection_records WHERE document=?1 AND scope=?2 AND key=?3",
                    params![document, scope, key],
                )
                .map_err(|e| e.to_string())?;
        }
        for (key, value) in rows {
            if old.get(&key) == Some(&value) {
                continue;
            }
            connection.execute("INSERT INTO projection_records(document,scope,key,schema,payload) VALUES(?1,?2,?3,1,?4) ON CONFLICT(document,scope,key) DO UPDATE SET schema=excluded.schema,payload=excluded.payload",
                params![document,scope,key,crate::database::encode(&value)]).map_err(|e|e.to_string())?;
        }
    }
    connection
        .execute("DELETE FROM documents WHERE name=?1", [document])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auxiliary_arrays_do_not_expand_the_streaming_view_header() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE documents(name TEXT PRIMARY KEY,payload TEXT);")
            .unwrap();
        db.execute_batch(include_str!("migrations/005.sql"))
            .unwrap();
        let mut before = json!({"id":1,"messages":[{"id":"answer","role":"Pi","text":"a"}],
            "queue_drafts":[[7,{"text":"queued ".repeat(40_000)}]],"skills":[{"name":"one","description":"skill ".repeat(40_000)}],"stats":{}});
        crate::history::stamp(&Value::Null, &mut before, 1);
        write(&db, "index.json", &json!({"threadViews":[before]})).unwrap();
        let header:String=db.query_row("SELECT payload FROM projection_records WHERE document='index.json' AND scope='threadViews/1' AND key='$'",[],|r|r.get(0)).unwrap();
        assert!(header.len() < 1024);
        let mut after = before.clone();
        after["messages"][0]["text"] = json!("ab");
        crate::history::stamp(&before, &mut after, 2);
        let changes = db.total_changes();
        write(
            &db,
            "index.json",
            &json!({"_partial":true,"threadViews":[after]}),
        )
        .unwrap();
        assert_eq!(
            db.total_changes() - changes,
            2,
            "only the message and its small view header change"
        );
        assert_eq!(
            read(&db, "index.json").unwrap().unwrap()["threadViews"][0],
            after
        );

        // Schema 1 already supports array rows and legacy inline view metadata.
        let mut legacy = after.clone();
        for field in crate::protocol::HISTORY_FIELDS {
            if legacy[field].is_array() {
                legacy[field] = json!([]);
            }
        }
        legacy.as_object_mut().unwrap().remove("_history");
        db.execute("UPDATE projection_records SET payload=?1 WHERE document='index.json' AND scope='threadViews/1' AND key='$'",[legacy.to_string()]).unwrap();
        db.execute("DELETE FROM projection_records WHERE document='index.json' AND scope='threadViews/1' AND key LIKE 'skills/%'",[]).unwrap();
        let restored = read(&db, "index.json").unwrap().unwrap()["threadViews"][0].clone();
        for (field, value) in after.as_object().unwrap() {
            assert!(
                restored[field] == *value,
                "legacy inline field {field} did not round-trip"
            );
        }
    }
    #[test]
    fn item_identity_sequence_and_order_survive_roundtrip_and_scoped_writes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE documents(name TEXT PRIMARY KEY,payload TEXT);")
            .unwrap();
        db.execute_batch(include_str!("migrations/005.sql"))
            .unwrap();
        let mut views = Vec::new();
        for id in 1..=500 {
            let mut view = json!({"id":id,"messages":[{"id":format!("user-{id}"),"role":"You","text":"old"},{"id":format!("answer-{id}"),"role":"Pi","text":"a"}],"stats":{}});
            crate::history::stamp(&Value::Null, &mut view, 1);
            views.push(view);
        }
        write(
            &db,
            "index.json",
            &json!({"version":4,"threads":[],"threadViews":views}),
        )
        .unwrap();
        let mut updated = views[0].clone();
        updated["messages"][1]["text"] = json!("ab");
        crate::history::stamp(&views[0], &mut updated, 2);
        let before = db.total_changes();
        write(
            &db,
            "index.json",
            &json!({"_partial":true,"threadViews":[updated]}),
        )
        .unwrap();
        assert_eq!(
            db.total_changes() - before,
            2,
            "only the item and thread high-water row change"
        );
        let restored = read(&db, "index.json").unwrap().unwrap();
        assert_eq!(restored["threadViews"][0], updated);
        assert_eq!(restored["threadViews"][499], views[499]);
        assert_eq!(
            restored["threadViews"][0]["_history"]["messages"][1]["sequence"],
            2
        );
        assert_eq!(
            restored["threadViews"][0]["_history"]["messages"][0]["sequence"],
            1
        );
        let mut reversed = updated.clone();
        reversed["messages"].as_array_mut().unwrap().reverse();
        crate::history::stamp(&updated, &mut reversed, 3);
        write(
            &db,
            "index.json",
            &json!({"_partial":true,"threadViews":[reversed]}),
        )
        .unwrap();
        assert_eq!(
            read(&db, "index.json").unwrap().unwrap()["threadViews"][0],
            reversed
        );
    }
    #[test]
    fn newer_row_schemas_fail_closed() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE documents(name TEXT PRIMARY KEY,payload TEXT);")
            .unwrap();
        db.execute_batch(include_str!("migrations/005.sql"))
            .unwrap();
        write(&db, "index.json", &json!({"version":4,"threads":[]})).unwrap();
        db.execute("UPDATE projection_records SET schema=2", [])
            .unwrap();
        assert!(read(&db, "index.json").unwrap_err().contains("newer Shina"));
    }
}
