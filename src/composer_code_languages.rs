//! Composer code-block languages, ported from KSG Code's `composer-code-languages.ts`:
//! the picker's catalog, its aliases, and the info-string helpers that keep a fence's
//! leading whitespace and trailing metadata when the language changes.

/// One catalog row: the id written to the info string, and its display label. The
/// empty id is plain text, written to the fence as no language at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeBlockLanguage {
    pub id: &'static str,
    pub label: &'static str,
}

/// KSG's `CODE_BLOCK_LANGUAGES`, same order and labels.
pub const CODE_BLOCK_LANGUAGES: &[CodeBlockLanguage] = &[
    CodeBlockLanguage {
        id: "",
        label: "Plain text",
    },
    CodeBlockLanguage {
        id: "asm",
        label: "Assembly",
    },
    CodeBlockLanguage {
        id: "bash",
        label: "Bash",
    },
    CodeBlockLanguage {
        id: "c",
        label: "C",
    },
    CodeBlockLanguage {
        id: "cpp",
        label: "C++",
    },
    CodeBlockLanguage {
        id: "csharp",
        label: "C#",
    },
    CodeBlockLanguage {
        id: "css",
        label: "CSS",
    },
    CodeBlockLanguage {
        id: "dart",
        label: "Dart",
    },
    CodeBlockLanguage {
        id: "diff",
        label: "Diff",
    },
    CodeBlockLanguage {
        id: "dockerfile",
        label: "Dockerfile",
    },
    CodeBlockLanguage {
        id: "elixir",
        label: "Elixir",
    },
    CodeBlockLanguage {
        id: "go",
        label: "Go",
    },
    CodeBlockLanguage {
        id: "graphql",
        label: "GraphQL",
    },
    CodeBlockLanguage {
        id: "haskell",
        label: "Haskell",
    },
    CodeBlockLanguage {
        id: "html",
        label: "HTML",
    },
    CodeBlockLanguage {
        id: "java",
        label: "Java",
    },
    CodeBlockLanguage {
        id: "javascript",
        label: "JavaScript",
    },
    CodeBlockLanguage {
        id: "json",
        label: "JSON",
    },
    CodeBlockLanguage {
        id: "jsx",
        label: "JSX",
    },
    CodeBlockLanguage {
        id: "kotlin",
        label: "Kotlin",
    },
    CodeBlockLanguage {
        id: "lua",
        label: "Lua",
    },
    CodeBlockLanguage {
        id: "markdown",
        label: "Markdown",
    },
    CodeBlockLanguage {
        id: "php",
        label: "PHP",
    },
    CodeBlockLanguage {
        id: "powershell",
        label: "PowerShell",
    },
    CodeBlockLanguage {
        id: "python",
        label: "Python",
    },
    CodeBlockLanguage {
        id: "r",
        label: "R",
    },
    CodeBlockLanguage {
        id: "ruby",
        label: "Ruby",
    },
    CodeBlockLanguage {
        id: "rust",
        label: "Rust",
    },
    CodeBlockLanguage {
        id: "scala",
        label: "Scala",
    },
    CodeBlockLanguage {
        id: "scss",
        label: "SCSS",
    },
    CodeBlockLanguage {
        id: "sql",
        label: "SQL",
    },
    CodeBlockLanguage {
        id: "svelte",
        label: "Svelte",
    },
    CodeBlockLanguage {
        id: "swift",
        label: "Swift",
    },
    CodeBlockLanguage {
        id: "toml",
        label: "TOML",
    },
    CodeBlockLanguage {
        id: "tsx",
        label: "TSX",
    },
    CodeBlockLanguage {
        id: "typescript",
        label: "TypeScript",
    },
    CodeBlockLanguage {
        id: "vue",
        label: "Vue",
    },
    CodeBlockLanguage {
        id: "xml",
        label: "XML",
    },
    CodeBlockLanguage {
        id: "yaml",
        label: "YAML",
    },
    CodeBlockLanguage {
        id: "zig",
        label: "Zig",
    },
];

/// KSG's `LANGUAGE_ALIASES`: lowercase alias to canonical id.
const LANGUAGE_ALIASES: &[(&str, &str)] = &[
    ("assembly", "asm"),
    ("cs", "csharp"),
    ("c++", "cpp"),
    ("js", "javascript"),
    ("md", "markdown"),
    ("plaintext", ""),
    ("py", "python"),
    ("rb", "ruby"),
    ("rs", "rust"),
    ("s", "asm"),
    ("sh", "bash"),
    ("shell", "bash"),
    ("text", ""),
    ("ts", "typescript"),
    ("txt", ""),
    ("yml", "yaml"),
];

/// KSG's `languageOfInfoString`: the first whitespace-separated token, or "".
pub fn language_of_info_string(info: &str) -> &str {
    info.split_whitespace().next().unwrap_or("")
}

/// KSG's `withInfoStringLanguage`: replace the leading language token and keep the
/// rest of the info string. Plain text (an empty `language`) clears the language
/// unless trailing metadata would then be parsed as one, which keeps `text` in front.
pub fn with_info_string_language(info: &str, language: &str) -> String {
    let leading = info.len() - info.trim_start().len();
    let rest = &info[leading + language_of_info_string(info).len()..];
    if !language.is_empty() {
        return format!("{}{language}{rest}", &info[..leading]);
    }
    if rest.trim().is_empty() {
        String::new()
    } else {
        format!("{}text{rest}", &info[..leading])
    }
}

/// KSG's `codeLanguageEntry`: resolve an id or alias, case-insensitive, to its row.
pub fn code_language_entry(language: &str) -> Option<&'static CodeBlockLanguage> {
    let lowered = language.to_lowercase();
    let id = LANGUAGE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lowered)
        .map(|(_, id)| *id)
        .unwrap_or(language);
    let lowered = id.to_lowercase();
    CODE_BLOCK_LANGUAGES
        .iter()
        .find(|entry| entry.id == lowered)
}

/// KSG's `codeLanguageLabel`: the catalog label, or the raw value when unknown.
pub fn code_language_label(language: &str) -> &str {
    code_language_entry(language).map_or(language, |entry| entry.label)
}

/// The picker's filter: label and id as in KSG, plus aliases so "yml" finds YAML and
/// "shell" finds Bash. `needle` is already trimmed and lowercased.
pub fn language_matches(id: &str, label: &str, needle: &str) -> bool {
    label.to_lowercase().contains(needle)
        || id.contains(needle)
        || LANGUAGE_ALIASES
            .iter()
            .any(|(alias, target)| *target == id && alias.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_of_info_string_takes_the_first_word() {
        assert_eq!(language_of_info_string(""), "");
        assert_eq!(language_of_info_string("rust"), "rust");
        assert_eq!(language_of_info_string("rs"), "rs");
        assert_eq!(language_of_info_string("Rust"), "Rust");
        assert_eq!(language_of_info_string("python title=main extra"), "python");
        assert_eq!(language_of_info_string("  cpp meta"), "cpp");
        assert_eq!(language_of_info_string("   "), "");
    }

    #[test]
    fn with_info_string_language_replaces_only_the_first_word() {
        assert_eq!(with_info_string_language("", "rust"), "rust");
        assert_eq!(with_info_string_language("rust", "go"), "go");
        assert_eq!(with_info_string_language("Rust", "go"), "go");
        assert_eq!(with_info_string_language("rs", "python"), "python");
        assert_eq!(
            with_info_string_language("rust title=x", "python"),
            "python title=x"
        );
        assert_eq!(
            with_info_string_language("  rust  meta", "go"),
            "  go  meta"
        );
    }

    #[test]
    fn with_info_string_language_plain_text_keeps_metadata_out_of_the_language_slot() {
        assert_eq!(with_info_string_language("rust", ""), "");
        assert_eq!(with_info_string_language("rust meta", ""), "text meta");
        assert_eq!(with_info_string_language("  rust", ""), "");
        assert_eq!(with_info_string_language("", "text"), "text");
    }

    #[test]
    fn code_language_entry_resolves_ids_and_aliases_case_insensitively() {
        assert_eq!(code_language_entry("rust").map(|e| e.id), Some("rust"));
        assert_eq!(code_language_entry("rs").map(|e| e.id), Some("rust"));
        assert_eq!(code_language_entry("Py").map(|e| e.id), Some("python"));
        assert_eq!(code_language_entry("c++").map(|e| e.id), Some("cpp"));
        assert_eq!(code_language_entry("sh").map(|e| e.id), Some("bash"));
        assert_eq!(code_language_entry("yml").map(|e| e.id), Some("yaml"));
        assert_eq!(code_language_entry("").map(|e| e.id), Some(""));
        assert_eq!(code_language_entry("text").map(|e| e.id), Some(""));
        assert_eq!(code_language_entry("cobol"), None);
        assert_eq!(code_language_label("rs"), "Rust");
        assert_eq!(code_language_label("cobol"), "cobol");
    }

    #[test]
    fn search_matches_label_id_and_alias() {
        assert!(language_matches("rust", "Rust", "rs"));
        assert!(language_matches("yaml", "YAML", "yml"));
        assert!(language_matches("bash", "Bash", "shell"));
        assert!(language_matches("cpp", "C++", "c++"));
        assert!(language_matches("", "Plain text", "txt"));
        assert!(language_matches("typescript", "TypeScript", "ts"));
        assert!(!language_matches("rust", "Rust", "cobol"));
    }
}
