-- The logical repository catalog. Repository identity is durable across
-- checkout moves and machines; checkout bindings map one repository to the
-- executor-local clones that serve it. No credentials, prompts or absolute
-- paths owned by another machine ever enter these rows.
CREATE TABLE catalog_repositories (
    repository_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    clone_url TEXT NOT NULL,
    base_branch TEXT,
    setup TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Every canonical remote ("host/path", lowercased host, one .git suffix
-- removed) names at most one repository. SSH and HTTPS spellings of one
-- remote collapse here; forks and other hosts stay distinct rows.
CREATE TABLE catalog_repository_remotes (
    canonical_remote TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES catalog_repositories(repository_id)
        ON DELETE CASCADE
);

-- A binding records which executor-local checkout serves a repository.
-- machine is 'local' or a linked server's stable id; project_id is that
-- executor's own numeric project key, meaningful only on the named machine.
CREATE TABLE catalog_checkouts (
    repository_id TEXT NOT NULL REFERENCES catalog_repositories(repository_id)
        ON DELETE CASCADE,
    machine TEXT NOT NULL,
    project_id INTEGER,
    path TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL CHECK(state IN ('bound','provisioning','failed')),
    reason TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(repository_id, machine, path)
);

-- Captain assignment is scope data: which logical repositories a captain may
-- delegate into, in the user's chosen order.
CREATE TABLE catalog_captain_scopes (
    captain INTEGER NOT NULL,
    repository_id TEXT NOT NULL REFERENCES catalog_repositories(repository_id)
        ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY(captain, repository_id)
);

PRAGMA user_version=10;
