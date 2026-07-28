use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use tree_sitter::{Language, Node, Parser};

use crate::workspace::{Workspace, WorkspaceError};

#[derive(Debug, Clone, Serialize)]
pub struct SyntaxSymbol {
    pub kind: String,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyntaxOutline {
    pub path: String,
    pub language: String,
    pub has_errors: bool,
    pub symbols: Vec<SyntaxSymbol>,
}

#[derive(Debug, Error)]
pub enum SyntaxError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("syntax analysis is not configured for {0}")]
    UnsupportedLanguage(String),
    #[error("could not configure the {0} parser")]
    Parser(String),
    #[error("tree-sitter could not parse {0}")]
    Parse(String),
}

/// Tree-sitter based structural outline for common systems, scripting, and web languages.
pub struct SyntaxAnalyzer;

impl SyntaxAnalyzer {
    pub fn outline(
        workspace: &Workspace,
        path: impl AsRef<Path>,
    ) -> Result<SyntaxOutline, SyntaxError> {
        let resolved = workspace.resolve_existing(path.as_ref())?;
        let source = workspace.read_text(&resolved)?;
        let extension = resolved
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let (language_name, language) = language_for_extension(extension)
            .ok_or_else(|| SyntaxError::UnsupportedLanguage(extension.to_owned()))?;

        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|_| SyntaxError::Parser(language_name.to_owned()))?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| SyntaxError::Parse(workspace.display_path(&resolved)))?;
        let root = tree.root_node();
        let mut symbols = Vec::new();
        collect_symbols(root, source.as_bytes(), &mut symbols);
        Ok(SyntaxOutline {
            path: workspace.display_path(resolved),
            language: language_name.to_owned(),
            has_errors: root.has_error(),
            symbols,
        })
    }
}

fn language_for_extension(extension: &str) -> Option<(&'static str, Language)> {
    match extension {
        "rs" => Some(("Rust", tree_sitter_rust::LANGUAGE.into())),
        "py" | "pyi" => Some(("Python", tree_sitter_python::LANGUAGE.into())),
        "js" | "mjs" | "cjs" | "jsx" => {
            Some(("JavaScript", tree_sitter_javascript::LANGUAGE.into()))
        }
        "ts" | "mts" | "cts" => Some((
            "TypeScript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        )),
        "tsx" => Some(("TSX", tree_sitter_typescript::LANGUAGE_TSX.into())),
        "go" => Some(("Go", tree_sitter_go::LANGUAGE.into())),
        _ => None,
    }
}

fn collect_symbols(node: Node<'_>, source: &[u8], output: &mut Vec<SyntaxSymbol>) {
    if is_symbol_kind(node.kind())
        && let Some(name) = symbol_name(node, source)
    {
        output.push(SyntaxSymbol {
            kind: node.kind().to_owned(),
            name,
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_symbols(child, source, output);
    }
}

fn symbol_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return name.utf8_text(source).ok().map(str::to_owned);
    }
    if node.kind() == "impl_item" {
        return node
            .child_by_field_name("type")
            .and_then(|name| name.utf8_text(source).ok())
            .map(str::to_owned);
    }
    None
}

fn is_symbol_kind(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "mod_item"
            | "const_item"
            | "static_item"
            | "type_item"
            | "macro_definition"
            | "function_definition"
            | "class_definition"
            | "function_declaration"
            | "class_declaration"
            | "method_definition"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration"
            | "method_declaration"
            | "type_declaration"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_rust_symbols() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let outline = SyntaxAnalyzer::outline(&workspace, "src/syntax.rs").unwrap();
        assert_eq!(outline.language, "Rust");
        assert!(
            outline
                .symbols
                .iter()
                .any(|symbol| symbol.name == "SyntaxAnalyzer")
        );
        assert!(!outline.has_errors);
    }

    #[test]
    fn includes_common_language_grammars() {
        assert_eq!(language_for_extension("py").unwrap().0, "Python");
        assert_eq!(language_for_extension("tsx").unwrap().0, "TSX");
        assert_eq!(language_for_extension("go").unwrap().0, "Go");
        assert!(language_for_extension("unknown").is_none());
    }

    #[test]
    fn outlines_python_symbols() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let outline = SyntaxAnalyzer::outline(&workspace, "tests/fixtures/sample.py").unwrap();
        assert_eq!(outline.language, "Python");
        assert!(
            outline
                .symbols
                .iter()
                .any(|symbol| symbol.name == "Greeter")
        );
        assert!(outline.symbols.iter().any(|symbol| symbol.name == "main"));
        assert!(!outline.has_errors);
    }
}
