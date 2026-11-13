#![allow(dead_code)] // Used by the diff panel, which lands in a later change.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::review_patch::{ChangeKind, FileDiff};

// Ported from KSG Code's `packages/shared/src/changedFileKind.ts` and
// `apps/web/src/components/diffs/diffFileTree.logic.ts`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangedFileKind {
    Generated,
    Mocks,
    Tests,
    Docs,
    Code,
}

const GENERATED_FILES: &[&str] = &[
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "composer.lock",
    "deno.lock",
    "flake.lock",
    "gemfile.lock",
    "go.sum",
    "gradle.lockfile",
    "mix.lock",
    "npm-shrinkwrap.json",
    "package-lock.json",
    "packages.lock.json",
    "pipfile.lock",
    "pnpm-lock.yaml",
    "podfile.lock",
    "poetry.lock",
    "pubspec.lock",
    "uv.lock",
    "yarn.lock",
];

const GENERATED_DIRS: &[&str] = &["__generated__", "_generated", "generated"];

const MOCK_WORDS: &[&str] = &[
    "mocks", "mock", "fixtures", "fixture", "stubs", "stub", "fakes", "fake",
];

const MOCK_DIRS: &[&str] = &[
    "__fixtures__",
    "__mocks__",
    "__snapshots__",
    "fakes",
    "fixture",
    "fixtures",
    "mock",
    "mocks",
    "stubs",
    "testdata",
];

const TEST_DIRS: &[&str] = &[
    "__tests__",
    "__test__",
    "cypress",
    "e2e",
    "evals",
    "spec",
    "specs",
    "test",
    "tests",
];

const CLASS_TEST_EXTENSIONS: &[&str] = &[
    "java", "kt", "kts", "scala", "groovy", "cs", "php", "swift", "m",
];

const DOCS_EXTENSIONS: &[&str] = &["md", "mdx", "markdown", "rst", "adoc", "txt"];

pub fn is_test_changed_file(path: &str) -> bool {
    matches!(
        classify_changed_file(path),
        ChangedFileKind::Tests | ChangedFileKind::Mocks
    )
}

pub fn classify_changed_file(path: &str) -> ChangedFileKind {
    let normalized = path.replace('\\', "/");
    let (dirs_str, name) = match normalized.rsplit_once('/') {
        Some((dirs, name)) => (Some(dirs), name),
        None => (None, normalized.as_str()),
    };

    let lower_name = name.to_ascii_lowercase();
    let in_dir = |targets: &[&str]| -> bool {
        let Some(dirs) = dirs_str else {
            return false;
        };
        dirs.split('/')
            .any(|segment| targets.contains(&segment.to_lowercase().as_str()))
    };

    if GENERATED_FILES.contains(&name.to_lowercase().as_str())
        || is_generated_name(name)
        || in_dir(GENERATED_DIRS)
    {
        return ChangedFileKind::Generated;
    }

    if is_mock_name(name) || in_dir(MOCK_DIRS) {
        return ChangedFileKind::Mocks;
    }

    if is_test_name(&lower_name) || is_class_test_name(name) || in_dir(TEST_DIRS) {
        return ChangedFileKind::Tests;
    }

    if is_docs_name(&lower_name) && !is_not_docs_name(&lower_name) {
        return ChangedFileKind::Docs;
    }

    ChangedFileKind::Code
}

fn is_generated_name(name: &str) -> bool {
    name.contains(".generated.")
        || name.contains(".gen.")
        || name.ends_with(".g.dart")
        || name.ends_with(".pb.go")
        || name.ends_with("_pb2.py")
        || name.ends_with("_pb2_grpc.py")
        || name.ends_with(".pb.cc")
        || name.ends_with(".pb.h")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".js.map")
        || name.ends_with(".css.map")
}

fn is_mock_name(name: &str) -> bool {
    if name.ends_with(".snap") {
        return true;
    }

    if let Some((stem, ext)) = name.rsplit_once('.')
        && !ext.is_empty()
        && MOCK_WORDS.iter().any(|word| {
            stem.strip_suffix(word)
                .is_some_and(|prefix| prefix.ends_with('.'))
        })
    {
        return true;
    }

    MOCK_WORDS.iter().any(|word| {
        let Some(rest) = name.strip_prefix(word) else {
            return false;
        };
        match rest.chars().next() {
            None => true,
            Some(ch) => matches!(ch, '-' | '_' | '.' | 'A'..='Z'),
        }
    })
}

fn is_test_name(lower_name: &str) -> bool {
    if let Some((stem, ext)) = lower_name.rsplit_once('.')
        && !ext.is_empty()
        && (stem.ends_with(".test")
            || stem.ends_with(".spec")
            || stem.ends_with(".e2e")
            || stem.ends_with(".eval")
            || stem.ends_with("_test")
            || stem.ends_with("_spec"))
    {
        return true;
    }

    if lower_name
        .strip_prefix("test_")
        .and_then(|s| s.strip_suffix(".py"))
        .is_some_and(|mid| !mid.is_empty())
    {
        return true;
    }

    if lower_name == "conftest.py" {
        return true;
    }

    lower_name
        .strip_prefix("vitest.setup.")
        .or_else(|| lower_name.strip_prefix("jest.setup."))
        .is_some_and(|ext| !ext.is_empty() && !ext.contains('.'))
}

fn is_class_test_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    CLASS_TEST_EXTENSIONS.contains(&ext)
        && (stem.ends_with("Test") || stem.ends_with("Tests") || stem.ends_with("Spec"))
}

fn is_docs_name(lower_name: &str) -> bool {
    lower_name
        .rsplit_once('.')
        .is_some_and(|(_, ext)| DOCS_EXTENSIONS.contains(&ext))
}

fn is_not_docs_name(lower_name: &str) -> bool {
    let Some(stem) = lower_name.strip_suffix(".txt") else {
        return false;
    };
    stem.starts_with("requirements")
        || stem.starts_with("constraints")
        || stem == "cmakelists"
        || stem == "robots"
        || stem == "llms"
}

// Part B: diff file tree ordering, ported from KSG Code's `diffFileTree.logic.ts`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub status: TreeStatus,
}

fn to_git_status(change: &ChangeKind) -> TreeStatus {
    match change {
        ChangeKind::Added | ChangeKind::Copied => TreeStatus::Added,
        ChangeKind::Deleted => TreeStatus::Deleted,
        ChangeKind::Modified => TreeStatus::Modified,
        ChangeKind::Renamed => TreeStatus::Renamed,
    }
}

pub fn tree_entries(files: &[FileDiff]) -> Vec<TreeEntry> {
    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut index_by_path: HashMap<&str, usize> = HashMap::new();

    for file in files {
        let status = to_git_status(&file.change);
        if let Some(&idx) = index_by_path.get(file.path.as_str()) {
            if let Some(entry) = entries.get_mut(idx)
                && entry.status != status
            {
                entry.status = TreeStatus::Modified;
            }
        } else {
            let idx = entries.len();
            index_by_path.insert(file.path.as_str(), idx);
            entries.push(TreeEntry {
                path: file.path.clone(),
                status,
            });
        }
    }

    entries
}

pub fn collect_directory_paths(paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut directories = Vec::new();

    for path in paths {
        let Some((dir_part, _)) = path.rsplit_once('/') else {
            continue;
        };
        let mut directory = String::new();
        for segment in dir_part.split('/') {
            directory.push_str(segment);
            directory.push('/');
            if seen.insert(directory.clone()) {
                directories.push(directory.clone());
            }
        }
    }

    directories
}

pub fn tree_positions(paths: &[String]) -> HashMap<String, usize> {
    let mut positions = HashMap::new();

    for (index, path) in paths.iter().enumerate() {
        positions.insert(path.clone(), index);
        let Some((dir_part, _)) = path.rsplit_once('/') else {
            continue;
        };
        let mut directory = String::new();
        for segment in dir_part.split('/') {
            directory.push_str(segment);
            directory.push('/');
            positions.entry(directory.clone()).or_insert(index);
        }
    }

    positions
}

pub fn order_like_tree(files: &[FileDiff]) -> Vec<FileDiff> {
    let paths: Vec<String> = files.iter().map(|file| file.path.clone()).collect();
    let positions = tree_positions(&paths);

    let mut keyed: Vec<(Vec<usize>, usize)> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let mut key = Vec::new();
            if let Some((dir_part, _)) = path.rsplit_once('/') {
                let mut directory = String::new();
                for segment in dir_part.split('/') {
                    directory.push_str(segment);
                    directory.push('/');
                    key.push(positions.get(&directory).copied().unwrap_or(usize::MAX));
                }
            }
            key.push(positions.get(path).copied().unwrap_or(usize::MAX));
            (key, index)
        })
        .collect();

    keyed.sort_by(|(left_key, _), (right_key, _)| left_key.cmp(right_key));

    keyed
        .into_iter()
        .filter_map(|(_, index)| files.get(index).cloned())
        .collect()
}

pub fn compare_tree_entries(a: &str, b: &str) -> Ordering {
    path_depth(a)
        .cmp(&path_depth(b))
        .then_with(|| locale_compare(a, b))
}

fn path_depth(path: &str) -> usize {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

// Primary UCA collation ranks matching JS `String.prototype.localeCompare`
// (default en-US locale, no options) for all ASCII bytes 0..=127.
// Control chars (0..=8, 14..=31, 127) occupy 1..=28; whitespace (9..=13) and
// punctuation/digits occupy 29..=76; `a..=z` and `A..=Z` share 77..=102.
const ASCII_PRIMARY_RANK: [u8; 128] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 29, 30, 31, 32, 33, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    22, 23, 24, 25, 26, 27, 34, 40, 44, 56, 66, 57, 55, 43, 45, 46, 52, 60, 37, 36, 42, 53, 67, 68,
    69, 70, 71, 72, 73, 74, 75, 76, 39, 38, 61, 62, 63, 41, 51, 77, 78, 79, 80, 81, 82, 83, 84, 85,
    86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 47, 54, 48, 59, 35, 58,
    77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99,
    100, 101, 102, 49, 64, 50, 65, 28,
];

fn ascii_primary_rank(byte: u8) -> u32 {
    u32::from(
        ASCII_PRIMARY_RANK
            .get(usize::from(byte))
            .copied()
            .unwrap_or(0),
    )
}

fn primary_keys(s: &str) -> impl Iterator<Item = u32> + '_ {
    s.chars().flat_map(|ch| {
        let mut buf = [0u32; 3];
        let mut len = 0usize;
        if ch.is_ascii() {
            buf = [ascii_primary_rank(ch as u8), 0, 0];
            len = 1;
        } else {
            for lower in ch.to_lowercase() {
                let rank = if lower.is_ascii() {
                    ascii_primary_rank(lower as u8)
                } else {
                    u32::from(lower).saturating_add(1000)
                };
                if let Some(slot) = buf.get_mut(len) {
                    *slot = rank;
                    len += 1;
                }
            }
        }
        buf.into_iter().take(len)
    })
}

fn locale_compare(a: &str, b: &str) -> Ordering {
    primary_keys(a).cmp(primary_keys(b)).then_with(|| {
        a.chars()
            .map(|ch| (!ch.is_lowercase(), ch))
            .cmp(b.chars().map(|ch| (!ch.is_lowercase(), ch)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksg_changed_file_kind_cases() {
        let cases = [
            ("src/contest.py", ChangedFileKind::Code),
            ("test/fixtures/pr.json", ChangedFileKind::Mocks),
            ("pnpm-lock.yaml", ChangedFileKind::Generated),
            ("convex/_generated/api.d.ts", ChangedFileKind::Generated),
            ("src/agent.eval.ts", ChangedFileKind::Tests),
            ("evals/title-generation.ts", ChangedFileKind::Tests),
            ("README.md", ChangedFileKind::Docs),
            ("tests/README.md", ChangedFileKind::Tests),
        ];
        for (path, expected) in cases {
            assert_eq!(classify_changed_file(path), expected, "path: {path}");
        }
    }

    #[test]
    fn generated_files_names_and_dirs() {
        let generated = [
            "bun.lock",
            "bun.lockb",
            "Cargo.lock",
            "composer.lock",
            "deno.lock",
            "flake.lock",
            "Gemfile.lock",
            "go.sum",
            "gradle.lockfile",
            "mix.lock",
            "npm-shrinkwrap.json",
            "package-lock.json",
            "packages.lock.json",
            "Pipfile.lock",
            "PNPM-LOCK.YAML",
            "Podfile.lock",
            "poetry.lock",
            "pubspec.lock",
            "uv.lock",
            "yarn.lock",
            "src/schema.generated.ts",
            "src/schema.gen.ts",
            "lib/user.g.dart",
            "proto/user.pb.go",
            "proto/user_pb2.py",
            "proto/user_pb2_grpc.py",
            "proto/user.pb.cc",
            "proto/user.pb.h",
            "dist/app.min.js",
            "dist/app.min.css",
            "dist/app.js.map",
            "dist/app.css.map",
            "__generated__/types.ts",
            "_generated/api.ts",
            "Generated/Models.cs",
        ];
        for path in generated {
            assert_eq!(
                classify_changed_file(path),
                ChangedFileKind::Generated,
                "expected generated: {path}"
            );
            assert!(!is_test_changed_file(path), "not test: {path}");
        }

        // GENERATED_NAME is case-sensitive.
        assert_eq!(
            classify_changed_file("proto/user.PB.go"),
            ChangedFileKind::Code
        );
        assert_eq!(
            classify_changed_file("src/schema.GENERATED.ts"),
            ChangedFileKind::Code
        );
    }

    #[test]
    fn is_test_changed_file_covers_every_mock_and_test_pattern() {
        let positives = [
            // MOCK_NAME
            "src/App.test.snap",
            "src/button.snap",
            "src/user.mock.ts",
            "src/user.mocks.json",
            "src/user.fixture.ts",
            "src/user.fixtures.json",
            "src/user.stub.ts",
            "src/user.stubs.json",
            "src/user.fake.ts",
            "src/user.fakes.json",
            "mock",
            "mocks",
            "mock-data.ts",
            "mock_data.ts",
            "mock.ts",
            "mocksUser.ts",
            "fixtureBuilder.ts",
            "stubData.rs",
            "fakeClock.ts",
            // MOCK_DIRS
            "__fixtures__/a.json",
            "__mocks__/fs.ts",
            "__snapshots__/btn.txt",
            "fakes/clock.ts",
            "fixture/user.json",
            "fixtures/user.json",
            "mock/server.ts",
            "Mocks/server.ts",
            "stubs/api.ts",
            "testdata/input.txt",
            // TEST_NAME
            "src/app.test.ts",
            "src/app.SPEC.tsx",
            "src/app.e2e.js",
            "src/agent.eval.py",
            "pkg/handler_test.go",
            "spec/handler_SPEC.rb",
            "tests_dir/test_auth.py",
            "TEST_AUTH.PY",
            "conftest.py",
            "Conftest.py",
            "vitest.setup.ts",
            "JEST.SETUP.js",
            // CLASS_TEST_NAME
            "src/FooTest.java",
            "src/FooTests.kt",
            "src/FooTest.kts",
            "src/FooSpec.scala",
            "src/FooTest.groovy",
            "src/FooTest.cs",
            "src/FooTest.php",
            "src/FooTests.swift",
            "src/FooSpec.m",
            // TEST_DIRS
            "__tests__/index.ts",
            "__test__/index.ts",
            "cypress/login.ts",
            "e2e/login.ts",
            "evals/prompt.ts",
            "spec/login.rb",
            "specs/login.rb",
            "test/helper.rs",
            "Tests/helper.rs",
        ];
        for path in positives {
            assert!(is_test_changed_file(path), "expected true: {path}");
        }

        let negatives = [
            "src/main.rs",
            "src/Latest.java",
            "src/Footest.java",
            "src/FooTest.JAVA",
            "src/FooTest.rs",
            "src/mockserver.ts",
            "src/MockData.ts",
            "src/foo.Mock.ts",
            "src/foo.mock.d.ts",
            "src/foo.mock.",
            "src/foo.test.d.ts",
            "src/foo.test.",
            "src/test.ts",
            "src/test_.py",
            "src/test_auth.rb",
            "src/my_conftest.py",
            "vitest.setup.",
            "vitest.setup.local.ts",
            "README.md",
            "requirements.txt",
            "pnpm-lock.yaml",
        ];
        for path in negatives {
            assert!(!is_test_changed_file(path), "expected false: {path}");
        }
    }

    #[test]
    fn docs_and_not_docs_and_precedence() {
        let docs = [
            "README.md",
            "docs/guide.MDX",
            "notes.markdown",
            "index.rst",
            "manual.adoc",
            "changelog.txt",
            "my-robots.txt",
            "requirements.md",
        ];
        for path in docs {
            assert_eq!(
                classify_changed_file(path),
                ChangedFileKind::Docs,
                "expected docs: {path}"
            );
        }

        let not_docs = [
            "requirements.txt",
            "requirements-dev.txt",
            "constraints.txt",
            "constraints-prod.txt",
            "CMakeLists.txt",
            "cmakelists.TXT",
            "robots.txt",
            "llms.txt",
        ];
        for path in not_docs {
            assert_eq!(
                classify_changed_file(path),
                ChangedFileKind::Code,
                "expected code: {path}"
            );
        }

        // Priority: generated > mocks > tests > docs > code.
        assert_eq!(
            classify_changed_file("generated/mocks/tests/README.md"),
            ChangedFileKind::Generated
        );
        assert_eq!(
            classify_changed_file("mocks/tests/README.md"),
            ChangedFileKind::Mocks
        );
        assert_eq!(
            classify_changed_file("tests/README.md"),
            ChangedFileKind::Tests
        );

        // Windows path separators are normalized.
        assert_eq!(
            classify_changed_file(r"test\fixtures\pr.json"),
            ChangedFileKind::Mocks
        );
        assert_eq!(
            classify_changed_file(r"src\app.test.ts"),
            ChangedFileKind::Tests
        );
    }

    fn file(change: ChangeKind, path: &str) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            previous_path: None,
            change,
            binary: false,
            old_mode: None,
            new_mode: None,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn ksg_tree_entries_folds_file_to_symlink_into_modified() {
        let entries = tree_entries(&[
            file(ChangeKind::Modified, "CLAUDE.md"),
            file(ChangeKind::Deleted, "AGENTS.md"),
            file(ChangeKind::Added, "AGENTS.md"),
            file(ChangeKind::Added, "docs/new.md"),
        ]);
        assert_eq!(
            entries,
            vec![
                TreeEntry {
                    path: "CLAUDE.md".to_string(),
                    status: TreeStatus::Modified,
                },
                TreeEntry {
                    path: "AGENTS.md".to_string(),
                    status: TreeStatus::Modified,
                },
                TreeEntry {
                    path: "docs/new.md".to_string(),
                    status: TreeStatus::Added,
                },
            ]
        );
    }

    #[test]
    fn tree_entries_maps_every_change_kind() {
        let entries = tree_entries(&[
            file(ChangeKind::Added, "a.rs"),
            file(ChangeKind::Deleted, "b.rs"),
            file(ChangeKind::Modified, "c.rs"),
            file(ChangeKind::Renamed, "d.rs"),
            file(ChangeKind::Copied, "e.rs"),
            // Same status twice stays unchanged.
            file(ChangeKind::Added, "a.rs"),
        ]);
        assert_eq!(
            entries,
            vec![
                TreeEntry {
                    path: "a.rs".to_string(),
                    status: TreeStatus::Added,
                },
                TreeEntry {
                    path: "b.rs".to_string(),
                    status: TreeStatus::Deleted,
                },
                TreeEntry {
                    path: "c.rs".to_string(),
                    status: TreeStatus::Modified,
                },
                TreeEntry {
                    path: "d.rs".to_string(),
                    status: TreeStatus::Renamed,
                },
                TreeEntry {
                    path: "e.rs".to_string(),
                    status: TreeStatus::Added,
                },
            ]
        );
    }

    #[test]
    fn collect_directory_paths_preserves_trailing_slashes_and_first_seen_order() {
        let paths = vec![
            "a/b/c.rs".to_string(),
            "a/d.rs".to_string(),
            "e.rs".to_string(),
        ];
        assert_eq!(
            collect_directory_paths(&paths),
            vec!["a/".to_string(), "a/b/".to_string()]
        );
    }

    #[test]
    fn ksg_order_like_tree_pulls_test_into_its_directory() {
        let ordered = order_like_tree(&[
            file(ChangeKind::Modified, "server/device/Host.ts"),
            file(ChangeKind::Modified, "web/settings.ts"),
            file(ChangeKind::Modified, "server/ws.ts"),
            file(ChangeKind::Modified, "server/device/Host.test.ts"),
            file(ChangeKind::Modified, "web/settings.test.ts"),
        ]);
        let names: Vec<&str> = ordered.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "server/device/Host.ts",
                "server/device/Host.test.ts",
                "server/ws.ts",
                "web/settings.ts",
                "web/settings.test.ts",
            ]
        );
    }

    #[test]
    fn order_like_tree_handles_three_levels_and_empty_slice() {
        assert!(order_like_tree(&[]).is_empty());

        let ordered = order_like_tree(&[
            file(ChangeKind::Modified, "src/ui/panel.rs"),
            file(ChangeKind::Modified, "README.md"),
            file(ChangeKind::Modified, "src/model.rs"),
            file(ChangeKind::Modified, "src/ui/widgets/button.rs"),
            file(ChangeKind::Modified, "docs/guide.md"),
            file(ChangeKind::Modified, "src/ui/widgets/label.rs"),
        ]);
        let names: Vec<&str> = ordered.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "src/ui/panel.rs",
                "src/ui/widgets/button.rs",
                "src/ui/widgets/label.rs",
                "src/model.rs",
                "README.md",
                "docs/guide.md",
            ]
        );
    }

    #[test]
    fn compare_tree_entries_matches_ksg_rules() {
        // Directory against a file at the same depth (depth 1): compared by name, not dir-first.
        assert_eq!(compare_tree_entries("docs/", "README.md"), Ordering::Less);
        assert_eq!(compare_tree_entries("src/", "README.md"), Ordering::Greater);
        assert_eq!(compare_tree_entries("foo", "foo/"), Ordering::Less);

        // Two files in different depths: shallower path comes first regardless of name.
        assert_eq!(compare_tree_entries("z.rs", "a/b.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("a/z.rs", "a/b/c.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("a/b.rs", "z.rs"), Ordering::Greater);

        // Names differing only by case: lowercase sorts before uppercase on tie,
        // while primary letter differences still take precedence.
        assert_eq!(compare_tree_entries("a.rs", "A.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("A.rs", "a.rs"), Ordering::Greater);
        assert_eq!(compare_tree_entries("aB.rs", "Ab.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("A.rs", "b.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("a/z.rs", "A/a.rs"), Ordering::Greater);

        // Punctuation and digits match JS default `localeCompare`.
        assert_eq!(compare_tree_entries("a_b", "a-b"), Ordering::Less);
        assert_eq!(compare_tree_entries("10", "2"), Ordering::Less);
        assert_eq!(compare_tree_entries("file10", "file2"), Ordering::Less);

        // Non-ASCII compares by lower-cased Unicode scalar values.
        assert_eq!(compare_tree_entries("α.rs", "β.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("Α.rs", "β.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("α.rs", "Α.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("a.rs", "α.rs"), Ordering::Less);
        assert_eq!(compare_tree_entries("a.rs", "a.rs"), Ordering::Equal);
    }

    #[test]
    fn tree_positions_records_first_seen_directory_and_latest_file_index() {
        let paths = vec![
            "a/b/c.rs".to_string(),
            "d.rs".to_string(),
            "a/e.rs".to_string(),
            "d.rs".to_string(),
        ];
        let positions = tree_positions(&paths);
        assert_eq!(positions.get("a/"), Some(&0));
        assert_eq!(positions.get("a/b/"), Some(&0));
        assert_eq!(positions.get("a/b/c.rs"), Some(&0));
        assert_eq!(positions.get("a/e.rs"), Some(&2));
        assert_eq!(positions.get("d.rs"), Some(&3));
    }
}
