# Stable workspace and owner identities

Schema 8 and index version 5 add a sidecar identity catalog. Existing numeric
Captain, thread and project keys, task and delivery IDs, command hashes,
document revisions, routine slots, provider sessions and peer references are
unchanged. The new UUIDs are additive aliases.

The catalog stores:

- `workspaceId`: one stable identity for this self-hosted Shina workspace.
- `ownerId`: the workspace's data owner. It is not a credential.
- `controllerId`: the existing persistent `server-id` that owns Captain intent.
- `executorId`: the existing `server-id` that currently runs the provider.
- Logical Captain, thread, project-checkout, task and routine IDs.
- Internal `mcp/captain/...` and `mcp/delegate/...` audit subjects mapped to
  their domain IDs. These subjects are never inserted into `devices` and
  cannot authenticate.

The executor initially equals the controller. The owner has a separate identity.
Execution leases, placement changes and run fencing are later work.

Authenticated `hello` exposes domain identities and logical record aliases.
Public `/v1/identity` remains a pairing-discovery response with the server ID,
machine label and protocol support. It does not enumerate workspace records.

## Migration rules

The SQLite migration first creates schema 8 during the existing pre-schema
backup flow. Identity data then commits in one transaction that also publishes
index version 5. A failed identity write leaves `identity_workspace` empty and
the legacy index version intact.

The migration chooses an integrity-checked schema 7 backup, records its path
and SHA-256 in `identity_workspace` and in the index `identity.migration`
object, and derives every logical ID from the workspace, controller, kind and
legacy key. Retrying after a failed transaction therefore reproduces the same
IDs. Duplicate local project paths share one checkout ID while both numeric
project aliases remain present.

Missing references, alias conflicts, malformed IDs and unknown future index or
schema versions block opening. File-only profiles use the same catalog in
`index.json`; their pre-migration directory is copied to `backups/` and the
new index is installed with the existing atomic JSON writer. Read-only
snapshot loading upgrades a legacy index in memory only.

## Restore a matched backup

1. Stop every desktop and server process using the profile. Do not copy or
   replace a live SQLite database.
2. Read the exact backup path from the profile:

   ```sh
   sqlite3 "$PROFILE/state.sqlite" \
     "SELECT backup_path, backup_sha256 FROM identity_workspace;"
   ```

3. Verify that the backup is intact and still schema 7:

   ```sh
   sqlite3 "$PROFILE/backups/<recorded-name>/state.sqlite" \
     "PRAGMA integrity_check; PRAGMA user_version;"
   ```

4. Move the failed top-level state files into a separate quarantine directory.
   Do not delete them. Then copy the matched backup files back:

   ```sh
   mkdir "$PROFILE/failed-identity-upgrade"
   mv "$PROFILE/state.sqlite" "$PROFILE/state.sqlite-wal" \
     "$PROFILE/state.sqlite-shm" "$PROFILE/index.json" \
     "$PROFILE/failed-identity-upgrade/" 2>/dev/null || true
   cp -a "$PROFILE/backups/<recorded-name>/." "$PROFILE/"
   ```

5. Run the previous binary against the restored schema 7/index version 4
   profile. Never run an older binary against schema 8 or index version 5.

The profile owner lock prevents concurrent writes. Backups are never selected
by age alone and are retained after a successful migration.
