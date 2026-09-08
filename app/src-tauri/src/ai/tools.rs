//! The tools the model may call against the scan.
//!
//! Every tool is read-only. The assistant advises; the user acts through
//! the right-click menu. Nothing here opens, moves or deletes anything,
//! and there is deliberately no tool that could.
//!
//! Each tool resolves a *path string* the model supplies. That string
//! crossed a network boundary and was written by a language model, so it
//! is matched against the tree by walking names from the scan root —
//! never handed to the filesystem, and never able to escape the scan.

use serde_json::{json, Value};
use st_core::digest;
use st_core::{NodeId, Tree};

use super::openrouter::{FunctionDef, ToolDef};

/// Row caps. Generous enough to answer real questions, small enough that
/// eight calls can't fill a modest context window.
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 40;

pub fn definitions() -> Vec<ToolDef> {
    let path_prop = json!({
        "type": "string",
        "description": "Folder path as shown in earlier results, e.g. \"C:/Users/you/Downloads\". Use \"/\" or the scan root name for the top level."
    });

    vec![
        tool(
            "list_folder",
            "List what is directly inside a folder, largest first. The first thing to reach for when asked what is using space somewhere.",
            json!({
                "type": "object",
                "properties": {
                    "path": path_prop,
                    "limit": {"type": "integer", "description": "Rows to return, default 20, max 40."}
                },
                "required": ["path"]
            }),
        ),
        tool(
            "find_large_files",
            "Find the biggest individual files anywhere beneath a folder, however deeply nested. Use when a folder is large but its immediate children are not.",
            json!({
                "type": "object",
                "properties": {
                    "path": path_prop,
                    "min_mb": {"type": "integer", "description": "Ignore files below this many MB. Default 100."},
                    "limit": {"type": "integer", "description": "Rows to return, default 20, max 40."}
                },
                "required": ["path"]
            }),
        ),
        tool(
            "folder_breakdown",
            "Total size per file extension beneath a folder. Use to answer what *kind* of data dominates.",
            json!({
                "type": "object",
                "properties": {
                    "path": path_prop,
                    "limit": {"type": "integer", "description": "Types to return, default 15, max 40."}
                },
                "required": ["path"]
            }),
        ),
        tool(
            "find_by_name",
            "Find every folder or file whose name contains a string, anywhere in the scan, with a combined total. Use for questions like how much all node_modules folders cost together.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Case-insensitive substring, e.g. \"node_modules\" or \".iso\"."},
                    "limit": {"type": "integer", "description": "Rows to return, default 20, max 40."}
                },
                "required": ["name"]
            }),
        ),
        tool(
            "find_reclaimable",
            "List recognised caches, build outputs and system folders with their sizes and a safety note for each. Call this first when asked what can be deleted — the safety notes are authoritative and must be repeated to the user rather than replaced with your own guess.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Candidates to return, default 20, max 40."}
                }
            }),
        ),
        tool(
            "folder_info",
            "Size, file count and any known safety note for one specific folder or file. Use to check a single item before recommending anything about it.",
            json!({
                "type": "object",
                "properties": {"path": path_prop},
                "required": ["path"]
            }),
        ),
    ]
}

fn tool(name: &'static str, description: &'static str, parameters: Value) -> ToolDef {
    ToolDef {
        def_type: "function",
        function: FunctionDef {
            name,
            description,
            parameters,
        },
    }
}

/// A short human-readable line describing what a call is about to do,
/// shown in the panel's trace so the user can see what was inspected.
pub fn describe(name: &str, args: &Value) -> String {
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    match name {
        "list_folder" => format!("Listing {}", display_path(path)),
        "find_large_files" => format!("Finding large files in {}", display_path(path)),
        "folder_breakdown" => format!("File types in {}", display_path(path)),
        "find_by_name" => format!(
            "Searching for \"{}\"",
            args.get("name").and_then(Value::as_str).unwrap_or("")
        ),
        "find_reclaimable" => "Checking for caches and build output".to_string(),
        "folder_info" => format!("Checking {}", display_path(path)),
        other => format!("Running {other}"),
    }
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        "the scan root".to_string()
    } else {
        path.to_string()
    }
}

/// Run one tool call. Errors come back as text for the model to read and
/// recover from — a wrong path is a normal event mid-conversation, not a
/// reason to end the turn.
pub fn dispatch(tree: &Tree, root: NodeId, use_alloc: bool, name: &str, args: &Value) -> String {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| (n as usize).clamp(1, MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT);

    match name {
        "find_reclaimable" => digest::reclaimable(tree, root, use_alloc, limit),
        "find_by_name" => match args.get("name").and_then(Value::as_str) {
            Some(needle) => digest::find_by_name(tree, root, needle, use_alloc, limit),
            None => "Error: `name` is required.".to_string(),
        },
        "list_folder" | "find_large_files" | "folder_breakdown" | "folder_info" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            let Some(node) = resolve(tree, root, path) else {
                return not_found(tree, root, path);
            };
            match name {
                "list_folder" => digest::folder_listing(tree, node, use_alloc, limit),
                "folder_breakdown" => digest::type_breakdown(tree, node, use_alloc, limit),
                "folder_info" => digest::node_detail(tree, node, use_alloc),
                _ => {
                    let min_mb = args.get("min_mb").and_then(Value::as_u64).unwrap_or(100);
                    digest::largest_files(tree, node, use_alloc, min_mb * 1024 * 1024, limit)
                }
            }
        }
        other => format!("Error: no tool named `{other}`."),
    }
}

/// A miss is common — models guess plausible-looking paths — so the reply
/// hands back the level that *does* exist rather than a bare failure,
/// which usually lets the next call succeed instead of derailing the turn.
fn not_found(tree: &Tree, root: NodeId, path: &str) -> String {
    format!(
        "Error: no folder matching \"{}\" in this scan. Here is the top level:\n\n{}",
        path,
        digest::folder_listing(tree, root, true, 20)
    )
}

/// Match a model-written path onto a node by walking names from the root.
///
/// Tolerant on purpose: either separator, an optional leading root name,
/// case-insensitive, and empty means the root. It resolves only by
/// walking the in-memory tree, so it cannot name anything outside the
/// scan no matter what the model writes.
fn resolve(tree: &Tree, root: NodeId, path: &str) -> Option<NodeId> {
    let cleaned = path.trim().replace('\\', "/");
    let mut segments: Vec<&str> = cleaned
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();

    if segments.is_empty() {
        return Some(root);
    }
    // Models tend to echo the root back ("C:/Users"); accept it either way.
    let root_name = tree.name(root).trim_end_matches(['/', '\\']).to_lowercase();
    if !root_name.is_empty() && segments[0].to_lowercase() == root_name {
        segments.remove(0);
    }

    let mut current = root;
    for segment in segments {
        let wanted = segment.to_lowercase();
        let next = tree
            .live_children(current)
            .into_iter()
            .find(|&child| tree.name(child).to_lowercase() == wanted)?;
        current = next;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use st_core::tree::{RawNode, TreeBuilder, ROOT};
    use st_core::NodeFlags;

    fn dir(parent: NodeId, name: &str) -> RawNode {
        RawNode {
            parent,
            name: name.to_string(),
            size_logical: 0,
            size_alloc: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        }
    }

    fn file(parent: NodeId, name: &str, size: u64) -> RawNode {
        RawNode {
            parent,
            name: name.to_string(),
            size_logical: size,
            size_alloc: size,
            mtime: 0,
            flags: NodeFlags::empty(),
        }
    }

    fn sample() -> (Tree, NodeId) {
        let mut b = TreeBuilder::new();
        let root = b.push(dir(ROOT, "C:"));
        let users = b.push(dir(root, "Users"));
        let downloads = b.push(dir(users, "Downloads"));
        b.push(file(downloads, "big.iso", 4_000_000_000));
        b.push(dir(root, "Windows"));
        (b.finalize(), root)
    }

    #[test]
    fn paths_resolve_with_either_separator_and_any_case() {
        let (tree, root) = sample();
        let expected = resolve(&tree, root, "Users/Downloads").unwrap();
        for variant in [
            "Users\\Downloads",
            "users/downloads",
            "C:/Users/Downloads",
            "c:\\users\\downloads",
            "/Users/Downloads/",
        ] {
            assert_eq!(
                resolve(&tree, root, variant),
                Some(expected),
                "failed to resolve {variant}"
            );
        }
    }

    #[test]
    fn an_empty_path_is_the_scan_root() {
        let (tree, root) = sample();
        assert_eq!(resolve(&tree, root, ""), Some(root));
        assert_eq!(resolve(&tree, root, "/"), Some(root));
    }

    #[test]
    fn a_path_leaving_the_scan_cannot_resolve() {
        let (tree, root) = sample();
        // Nothing the model writes may name something outside the scan.
        for escape in [
            "..",
            "../../etc/passwd",
            "Users/../../Windows",
            "/etc/passwd",
        ] {
            assert_eq!(
                resolve(&tree, root, escape),
                None,
                "{escape} must not resolve"
            );
        }
    }

    #[test]
    fn an_unknown_path_returns_guidance_not_just_an_error() {
        let (tree, root) = sample();
        let out = dispatch(
            &tree,
            root,
            true,
            "list_folder",
            &json!({"path": "Nope/Here"}),
        );
        assert!(out.starts_with("Error:"));
        assert!(
            out.contains("Users"),
            "the reply should show what does exist so the model can recover:\n{out}"
        );
    }

    #[test]
    fn limits_are_clamped_rather_than_trusted() {
        let (tree, root) = sample();
        // A model asking for 100000 rows must not get them.
        let out = dispatch(
            &tree,
            root,
            true,
            "list_folder",
            &json!({"path": "", "limit": 100_000}),
        );
        assert!(out.len() < digest::MAX_RESULT_CHARS + 100);
    }

    #[test]
    fn an_unknown_tool_name_is_reported_not_panicked_on() {
        let (tree, root) = sample();
        let out = dispatch(&tree, root, true, "delete_everything", &json!({}));
        assert!(out.contains("no tool named"));
    }

    #[test]
    fn there_is_no_tool_that_modifies_anything() {
        // The assistant is advice-only by design. If a future change adds
        // a mutating tool, this test should be the thing that stops it
        // going in unnoticed.
        let names: Vec<&str> = definitions().iter().map(|t| t.function.name).collect();
        for name in &names {
            assert!(
                name.starts_with("list_")
                    || name.starts_with("find_")
                    || name.starts_with("folder_"),
                "{name} does not look read-only"
            );
        }
        assert_eq!(
            names.len(),
            6,
            "tool count changed — re-check read-only-ness"
        );
    }

    #[test]
    fn descriptions_read_as_plain_english_for_the_trace() {
        let d = describe("list_folder", &json!({"path": "C:/Users"}));
        assert_eq!(d, "Listing C:/Users");
        assert_eq!(describe("list_folder", &json!({})), "Listing the scan root");
        assert_eq!(
            describe("find_reclaimable", &json!({})),
            "Checking for caches and build output"
        );
    }

    #[test]
    fn find_large_files_defaults_to_a_sensible_floor() {
        let (tree, root) = sample();
        let out = dispatch(&tree, root, true, "find_large_files", &json!({"path": ""}));
        assert!(out.contains("big.iso"), "{out}");
    }
}
