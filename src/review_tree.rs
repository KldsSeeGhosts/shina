#![allow(dead_code)] // Used by the diff panel, which lands in a later change.

// Ported from KSG Code's `packages/shared/src/changedFileKind.ts`.

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
}
