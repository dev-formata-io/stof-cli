//
// Copyright 2025 Formata, Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

//! `stof lsp`: a language server over stdio (diagnostics, outline, go to definition, hover, completion,
//! references, and rename).
//!
//! Every change re-parses the document with `parse_with_info`, which records where everything is defined.
//! A document in a package (a pkg.stof in its directory or above) is parsed with its whole package, so
//! types and fields from other files resolve; open documents are parsed from the editor, not the disk.
//! The parse is sandboxed: no network or environment, file reads only (for imports), init functions don't
//! run, and code that runs while parsing (field values) has a short time limit. While a document doesn't
//! parse (Ex. mid-edit), navigation and completion use its last good parse.

use std::{collections::HashMap, ops::Deref, path::Path, time::Duration};
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Notification as _, PublishDiagnostics}, request::{Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, References, Rename, Request as _}, CompletionItem, CompletionItemKind, CompletionOptions, CompletionResponse, Diagnostic, DiagnosticSeverity, DocumentSymbol, DocumentSymbolResponse, Documentation, GotoDefinitionResponse, Hover, HoverContents, HoverProviderCapability, InitializeParams, Location, MarkupContent, MarkupKind, OneOf, Position, PublishDiagnosticsParams, Range, ServerCapabilities, SymbolKind, TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Url, WorkspaceEdit};
use stof::{rustc_hash::FxHashMap, model::{filesys::fs_library, DataRef, Field, Func, Graph, NodeRef, Profile, Prototype, SPath}, parser::{context::resolve_import_path, info::{parse_with_info, Def, DefKind, ParseInfo, Warning}, source::SrcLoc}, runtime::{Error, Type, Val}};


/// An open document and what parsing it found.
struct Doc {
    uri: Url,
    text: String,
    path: String,
    graph: Graph,
    info: ParseInfo,
    /// The graph and definitions are from a parse without errors (this one or an earlier one).
    good: bool,
}


/// Run the language server on stdin/stdout until the client shuts it down.
pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    colored::control::set_override(false); // plain messages
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::FULL),
            save: Some(TextDocumentSyncSaveOptions::Supported(true)),
            ..Default::default()
        })),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions { trigger_characters: Some(vec![".".into(), ":".into(), "<".into()]), ..Default::default() }),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        ..Default::default()
    };
    let _params: InitializeParams = serde_json::from_value(connection.initialize(serde_json::to_value(capabilities)?)?)?;

    let mut docs: HashMap<Url, Doc> = HashMap::new();
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? { break; }
                let response = handle_request(&docs, request);
                connection.sender.send(Message::Response(response))?;
            },
            Message::Notification(notification) => {
                let changed = match notification.method.as_str() {
                    DidOpenTextDocument::METHOD => {
                        let params: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(notification.params)?;
                        Some((params.text_document.uri, params.text_document.text))
                    },
                    DidChangeTextDocument::METHOD => {
                        let mut params: lsp_types::DidChangeTextDocumentParams = serde_json::from_value(notification.params)?;
                        params.content_changes.pop().map(|change| (params.text_document.uri, change.text)) // full sync: the last change is the text
                    },
                    DidSaveTextDocument::METHOD => {
                        // other open documents may use what changed (Ex. a type in this file): parse them again
                        let params: lsp_types::DidSaveTextDocumentParams = serde_json::from_value(notification.params)?;
                        let others: Vec<Url> = docs.keys().filter(|uri| **uri != params.text_document.uri).cloned().collect();
                        for uri in others {
                            let Some(previous) = docs.remove(&uri) else { continue; };
                            let open = docs.values().map(|doc| (doc.path.clone(), doc.text.clone())).collect();
                            let doc = analyze(&uri, previous.text.clone(), Some(previous), open);
                            publish(&connection, uri.clone(), diagnostics(&doc))?;
                            docs.insert(uri, doc);
                        }
                        None
                    },
                    DidCloseTextDocument::METHOD => {
                        let params: lsp_types::DidCloseTextDocumentParams = serde_json::from_value(notification.params)?;
                        docs.remove(&params.text_document.uri);
                        publish(&connection, params.text_document.uri, vec![])?;
                        None
                    },
                    _ => None,
                };
                if let Some((uri, text)) = changed {
                    let previous = docs.remove(&uri);
                    let open = docs.values().map(|doc| (doc.path.clone(), doc.text.clone())).collect();
                    let doc = analyze(&uri, text, previous, open);
                    let diagnostics = diagnostics(&doc);
                    docs.insert(uri.clone(), doc);
                    publish(&connection, uri, diagnostics)?;
                }
            },
            Message::Response(_) => {},
        }
    }
    drop(connection); // closes the writer thread
    io_threads.join()?;
    Ok(())
}


/// Answer a request (null for anything unknown or not found).
fn handle_request(docs: &HashMap<Url, Doc>, request: Request) -> Response {
    let result = match request.method.as_str() {
        DocumentSymbolRequest::METHOD => {
            let params: Option<lsp_types::DocumentSymbolParams> = serde_json::from_value(request.params).ok();
            params.and_then(|params| docs.get(&params.text_document.uri))
                .map(|doc| serde_json::to_value(DocumentSymbolResponse::Nested(outline(doc))).unwrap_or_default())
        },
        GotoDefinition::METHOD => {
            let params: Option<lsp_types::GotoDefinitionParams> = serde_json::from_value(request.params).ok();
            params.and_then(|params| definition(docs, &params.text_document_position_params))
                .map(|location| serde_json::to_value(GotoDefinitionResponse::Scalar(location)).unwrap_or_default())
        },
        HoverRequest::METHOD => {
            let params: Option<lsp_types::HoverParams> = serde_json::from_value(request.params).ok();
            params.and_then(|params| hover(docs, &params.text_document_position_params))
                .map(|hover| serde_json::to_value(hover).unwrap_or_default())
        },
        Completion::METHOD => {
            let params: Option<lsp_types::CompletionParams> = serde_json::from_value(request.params).ok();
            params.and_then(|params| completion(docs, &params.text_document_position))
                .map(|items| serde_json::to_value(CompletionResponse::Array(items)).unwrap_or_default())
        },
        References::METHOD => {
            let params: Option<lsp_types::ReferenceParams> = serde_json::from_value(request.params).ok();
            params.and_then(|params| references(docs, &params.text_document_position, params.context.include_declaration))
                .map(|locations| serde_json::to_value(locations).unwrap_or_default())
        },
        Rename::METHOD => {
            let params: Option<lsp_types::RenameParams> = serde_json::from_value(request.params).ok();
            params.and_then(|params| rename(docs, &params.text_document_position, &params.new_name))
                .map(|edit| serde_json::to_value(edit).unwrap_or_default())
        },
        _ => None,
    };
    Response::new_ok(request.id, result.unwrap_or(serde_json::Value::Null))
}


fn publish(connection: &Connection, uri: Url, diagnostics: Vec<Diagnostic>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
    connection.sender.send(Message::Notification(Notification::new(PublishDiagnostics::METHOD.into(), params)))?;
    Ok(())
}


/// Parse a document in a sandbox, recording definitions, warnings, and the error (if any).
/// In a package, the whole package is parsed (with the open documents' texts) so other files' types resolve.
/// When it doesn't parse, the definitions (and graph) of its last good parse are kept for navigation.
fn analyze(uri: &Url, text: String, previous: Option<Doc>, mut open: FxHashMap<String, String>) -> Doc {
    let path = uri.to_file_path().map(|path| path.to_string_lossy().to_string()).unwrap_or_else(|_| uri.path().to_string());
    let path = resolve_import_path(None, "", &path).ok().and_then(|path| path.into_os_string().into_string().ok()).unwrap_or(path); // same form as imports
    open.insert(path.clone(), text.clone());

    let parse = |format: &str, file: &str, open: FxHashMap<String, String>| {
        let mut graph = Graph::default();
        graph.set_deadpools_enabled(false);
        fs_library(&mut graph);
        if let Some(fs) = graph.libfuncs.get_mut("fs") {
            fs.retain(|name, _| name.starts_with("read")); // imports read files; nothing writes
        }
        graph.max_execution_time = Some(Duration::from_secs(2));
        let profile = Profile { debug_info: false, ..Profile::test() }; // everything, including #[test] functions
        let (res, info) = parse_with_info(&mut graph, format, file, open, profile);
        (graph, res, info)
    };

    // With its package if it's in one (unless the package parses fine without it), else on its own
    let mut parsed = None;
    if let Some(package) = package_of(&path) {
        let (graph, res, info) = parse("pkg", &package, open.clone());
        if res.is_err() || info.files.contains(&path) { parsed = Some((graph, res, info)); }
    }
    let (graph, res, mut info) = parsed.unwrap_or_else(|| parse("stof", &path, open));

    // A parse error replaces the warnings (shown as an error, see `diagnostics`)
    if let Err(error) = res {
        let (message, loc) = match &error {
            Error::ParseError(error) => {
                let file = error.file_path().unwrap_or(path.as_str());
                let loc = error.location().map(|(line, col)| SrcLoc { file: Some(file.into()), line: line as u32, col: col as u32 });
                (error.describe(), loc)
            },
            error => (error.to_string(), None),
        };
        let warnings = vec![Warning { message: format!("error: {message}"), loc, file: Some(path.clone()) }];
        if let Some(previous) = previous.filter(|previous| previous.good) {
            return Doc { uri: uri.clone(), text, path, graph: previous.graph, info: ParseInfo { warnings, ..previous.info }, good: true };
        }
        info.warnings = warnings;
        return Doc { uri: uri.clone(), text, path, graph, info, good: false };
    }
    Doc { uri: uri.clone(), text, path, graph, info, good: true }
}


/// The package a file is in: the nearest pkg.stof in its directory or above.
fn package_of(path: &str) -> Option<String> {
    let mut dir = Path::new(path).parent();
    while let Some(current) = dir {
        let package = current.join("pkg.stof");
        if package.is_file() && package.to_string_lossy() != path { return Some(package.to_string_lossy().to_string()); }
        dir = current.parent();
    }
    None
}


/// Diagnostics for a document: its parse error, or the warnings found in it.
fn diagnostics(doc: &Doc) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for warning in &doc.info.warnings {
        let (severity, message) = match warning.message.strip_prefix("error: ") {
            Some(message) => (DiagnosticSeverity::ERROR, message.to_string()),
            None => (DiagnosticSeverity::WARNING, warning.message.clone()),
        };
        let range = match &warning.loc {
            Some(loc) if loc.file.as_deref() == Some(doc.path.as_str()) => {
                let line = line_of(&doc.text, loc.line);
                // parse errors count columns in chars, warnings in bytes
                let start = if severity == DiagnosticSeverity::ERROR { char_utf16(line, loc.col) } else { byte_utf16(line, loc.col) };
                let word = line.get(byte_index(line, start)..).map(|rest| rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').map(|c| c.len_utf16() as u32).sum::<u32>()).unwrap_or(0);
                Range::new(Position::new(loc.line - 1, start), Position::new(loc.line - 1, start + word.max(1)))
            },
            Some(_) if severity != DiagnosticSeverity::ERROR => continue, // a warning in another file
            Some(loc) => {
                // an error in another file: show it at the top of this one
                let message = format!("{} ({})", message, loc.display());
                diagnostics.push(Diagnostic { range: Range::default(), severity: Some(severity), source: Some("stof".into()), message, ..Default::default() });
                continue;
            },
            None if warning.file.as_deref() == Some(doc.path.as_str()) => Range::default(),
            None => continue, // about another file
        };
        diagnostics.push(Diagnostic { range, severity: Some(severity), source: Some("stof".into()), message, ..Default::default() });
    }
    diagnostics
}


/// Outline: roots, fields, functions, and imports defined in this document, nested by where they are.
fn outline(doc: &Doc) -> Vec<DocumentSymbol> {
    let mut defs: Vec<&Def> = doc.info.defs.iter().filter(|def| def.start.file.as_deref() == Some(doc.path.as_str())).collect();
    defs.sort_by_key(|def| (def.start.line, def.start.col));

    // stack of open symbols (with where they end): each def goes inside the last one that contains it
    let mut result = Vec::new();
    let mut stack: Vec<(DocumentSymbol, Position)> = Vec::new();
    for def in defs {
        let selection = name_range(&doc.text, def);
        let end = def.end.as_ref().map(|end| Position::new(end.line - 1, byte_utf16(line_of(&doc.text, end.line), end.col))).unwrap_or(selection.end);
        while let Some((_, open_end)) = stack.last() {
            if *open_end > selection.start { break; }
            close_symbol(&mut stack, &mut result);
        }
        #[allow(deprecated)]
        let symbol = DocumentSymbol {
            name: if def.kind == DefKind::Import { format!("import '{}'", def.name) } else { def.name.clone() },
            detail: detail(&doc.graph, def),
            kind: symbol_kind(&doc.graph, def),
            tags: None,
            deprecated: None,
            range: Range::new(selection.start, end.max(selection.end)),
            selection_range: selection,
            children: None,
        };
        stack.push((symbol, end));
    }
    while !stack.is_empty() { close_symbol(&mut stack, &mut result); }
    result
}

fn close_symbol(stack: &mut Vec<(DocumentSymbol, Position)>, result: &mut Vec<DocumentSymbol>) {
    if let Some((symbol, _)) = stack.pop() {
        match stack.last_mut() {
            Some((parent, _)) => parent.children.get_or_insert_with(Vec::new).push(symbol),
            None => result.push(symbol),
        }
    }
}

fn symbol_kind(graph: &Graph, def: &Def) -> SymbolKind {
    match def.kind {
        DefKind::Root => SymbolKind::MODULE,
        DefKind::Import => SymbolKind::FILE,
        DefKind::Func if def.parent.node_parent(graph).is_some() => SymbolKind::METHOD,
        DefKind::Func => SymbolKind::FUNCTION,
        DefKind::Field => match &def.obj {
            Some(obj) if is_type(graph, obj) => SymbolKind::CLASS,
            Some(_) => SymbolKind::OBJECT,
            None => SymbolKind::FIELD,
        },
    }
}

fn is_type(graph: &Graph, node: &NodeRef) -> bool {
    node.node(graph).map(|node| node.attributes.contains_key("type")).unwrap_or(false)
}


/// Short detail for the outline: a function's signature, or a field's type.
fn detail(graph: &Graph, def: &Def) -> Option<String> {
    let data = def.data.as_ref()?;
    if let Some(func) = graph.get_stof_data::<Func>(data) {
        return Some(func_signature(graph, func, None));
    }
    if def.obj.as_ref().map(|obj| is_type(graph, obj)).unwrap_or(false) { return Some("type".into()); }
    graph.get_stof_data::<Field>(data).map(|field| field_type(graph, field))
}

fn func_signature(graph: &Graph, func: &Func, name: Option<&str>) -> String {
    let params = func.params.iter().map(|param| format!("{}: {}", param.name.as_ref(), param.param_type.rt_type_of(graph))).collect::<Vec<_>>().join(", ");
    let mut signature = format!("fn {}({params})", name.unwrap_or(""));
    if func.attributes.contains_key("async") { signature.insert_str(0, "async "); }
    if func.return_type != Type::Void { signature.push_str(&format!(" -> {}", func.return_type.rt_type_of(graph))); }
    signature
}

fn field_type(graph: &Graph, field: &Field) -> String {
    match &field.value.vtype {
        Some(vtype) => vtype.rt_type_of(graph).to_string(),
        None => field.value.val.read().spec_type(graph).rt_type_of(graph).to_string(),
    }
}


/// Range of a definition's name (import: the quoted path).
fn name_range(text: &str, def: &Def) -> Range {
    let line = line_of(text, def.start.line);
    let start = byte_utf16(line, def.start.col);
    let at = line.get((def.start.col as usize).saturating_sub(1)..).unwrap_or("");
    let quoted = at.starts_with('"') || at.starts_with('\'');
    let len = def.name.encode_utf16().count() as u32 + if quoted { 2 } else { 0 };
    Range::new(Position::new(def.start.line - 1, start), Position::new(def.start.line - 1, start + len))
}


/// What the cursor is on.
enum Target {
    /// A definition (index in the document's defs).
    Def(usize),
    /// A library function ("Lib.func").
    Lib(String),
    /// A local variable or parameter, and where it's declared (0-based line, byte column).
    Local { name: String, line: usize, col: usize },
}


fn is_word(c: char) -> bool { c.is_alphanumeric() || c == '_' }

/// Is a definition in this document (not an imported file)?
fn in_doc(doc: &Doc, def: &Def) -> bool {
    def.start.file.as_deref() == Some(doc.path.as_str())
}

/// What `self` is at a position (1-based line and byte column, like SrcLoc), and the function around it.
fn scope<'a>(doc: &'a Doc, here: (u32, u32)) -> (Option<NodeRef>, Option<&'a Def>) {
    let start = |def: &Def| (def.start.line, def.start.col);
    let mut innermost: Option<&Def> = None;
    let mut func: Option<&Def> = None;
    for def in doc.info.defs.iter().filter(|def| in_doc(doc, def)) {
        let end = def.end.as_ref().map(|end| (end.line, end.col)).unwrap_or(start(def));
        if !(start(def) <= here && here < end) { continue; }
        if innermost.map(|inner| start(def) > start(inner)).unwrap_or(true) { innermost = Some(def); }
        if def.kind == DefKind::Func && func.map(|inner| start(def) > start(inner)).unwrap_or(true) { func = Some(def); }
    }
    let self_node = match innermost {
        Some(def) => match (&def.kind, &def.obj) {
            (DefKind::Field | DefKind::Root, Some(obj)) => Some(obj.clone()),
            _ => Some(def.parent.clone()),
        },
        // top level: the object the file is parsed into (what its first definition is on)
        None => doc.info.defs.iter().filter(|def| in_doc(doc, def)).min_by_key(|def| start(def)).map(|def| def.parent.clone()).or_else(|| doc.graph.main_root()),
    };
    (self_node, func)
}


/// Words (identifiers, numbers) in a line: (byte start, word).
fn words(line: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut start = None;
    for (index, c) in line.char_indices().chain(std::iter::once((line.len(), ' '))) {
        match (start, is_word(c)) {
            (None, true) => start = Some(index),
            (Some(begin), false) => { words.push((begin, &line[begin..index])); start = None; },
            _ => {},
        }
    }
    words
}

/// Local variables and parameters of a function declared before a position (0-based line, byte column):
/// (name, line, column), in order. Parameters are the names before a ':' in the signature; variables
/// follow let, const, or using (or are in a destructuring pattern after let or const: `let { a, b: c } = ..`);
/// and a catch can name its error.
fn locals(doc: &Doc, func: &Def, until: (usize, usize)) -> Vec<(String, usize, usize)> {
    let mut found = Vec::new();
    let first = func.start.line as usize - 1;
    let mut in_signature = true; // until the body's '{'
    let mut pattern: Option<PatternScan> = None; // in a destructuring pattern
    for (index, line) in doc.text.lines().enumerate().skip(first) {
        if index > until.0 { break; }
        let line = line.trim_end_matches('\r');
        let from = if index == first { func.start.col as usize - 1 } else { 0 };
        let mut previous: (usize, &str) = (from, ""); // previous word (its end, text)
        for (start, word) in words(line) {
            if start < from || (index, start) >= until { continue; }
            let gap = &line[previous.0..start];
            if in_signature && gap.contains('{') { in_signature = false; }
            if matches!(previous.1, "let" | "const") && gap.trim_start().starts_with(['{', '[', '(']) {
                pattern = Some(PatternScan::default());
            }
            if let Some(scan) = &mut pattern {
                if !scan.read(gap) { pattern = None; }
            }
            let next = line[start + word.len()..].trim_start();
            let declared = match &pattern {
                // a name in a pattern, unless it's a field being renamed ("b" in "b: c") or in a default value
                Some(scan) => scan.default_at.is_none() && !next.starts_with(':') && word != "_" && !word.starts_with(|c: char| c.is_ascii_digit()),
                None => matches!(previous.1, "let" | "const" | "using") && gap.trim().is_empty()
                    || previous.1 == "catch" && gap.trim() == "("
                    || in_signature && (next.starts_with(':') || next.starts_with("?:")) && !next.starts_with("::"),
            };
            if declared { found.push((word.to_string(), index, start)); }
            previous = (start + word.len(), word);
        }
        if in_signature && line[previous.0..].contains('{') { in_signature = false; }
        if matches!(previous.1, "let" | "const") && line[previous.0..].trim_start().starts_with(['{', '[', '(']) {
            pattern = Some(PatternScan::default()); // a pattern starting at the end of the line
        }
        if let Some(scan) = &mut pattern {
            if !scan.read(&line[previous.0..]) { pattern = None; }
        }
    }
    found
}

#[derive(Default)]
/// Reading a destructuring pattern: how deep in its brackets, and where a default value started.
struct PatternScan {
    depth: i32,
    default_at: Option<i32>,
}
impl PatternScan {
    /// Read the text between words; false once the pattern is closed.
    fn read(&mut self, text: &str) -> bool {
        for c in text.chars() {
            match c {
                '{' | '[' | '(' => self.depth += 1,
                '}' | ']' | ')' => {
                    self.depth -= 1;
                    if self.default_at.map(|at| self.depth < at).unwrap_or(false) { self.default_at = None; }
                    if self.depth <= 0 { return false; }
                },
                ',' if self.default_at == Some(self.depth) => self.default_at = None,
                '=' if self.default_at.is_none() => self.default_at = Some(self.depth),
                _ => {},
            }
        }
        true
    }
}


/// Find what's under the cursor: a definition (by path, type name, or import), a local variable, or a
/// library function.
fn target(doc: &Doc, position: &Position) -> Option<Target> {
    let line = doc.text.lines().nth(position.line as usize)?.trim_end_matches('\r');
    let cursor = byte_index(line, position.character);
    let here = (position.line + 1, cursor as u32 + 1); // 1-based line and byte column, like SrcLoc

    // On a definition's name (or an import's path)?
    for (index, def) in doc.info.defs.iter().enumerate().filter(|(_, def)| in_doc(doc, def)) {
        let range = name_range(&doc.text, def);
        if range.start.line == position.line && range.start <= *position && *position <= range.end {
            return Some(Target::Def(index));
        }
    }

    // The path under the cursor, up to the end of the word the cursor is on (Ex. "self.a" on "a" in "self.a.b")
    let end = cursor + line[cursor..].find(|c: char| !is_word(c)).unwrap_or(line.len() - cursor);
    let start = line[..end].rfind(|c: char| !(is_word(c) || c == '.' || c == ':')).map(|index| index + 1).unwrap_or(0);
    let path = line[start..end].trim_start_matches(['.', ':']);
    if path.is_empty() { return None; }

    // Library function? (Lib::func, Lib.func, or a bare standard library call)
    let graph = &doc.graph;
    if let Some((lib, func)) = path.split_once("::").or_else(|| path.split_once('.')) {
        if graph.libfuncs.get(lib).map(|funcs| funcs.contains_key(func)).unwrap_or(false) && (path.contains("::") || graph.find_root_named(lib).is_none()) {
            return Some(Target::Lib(format!("{lib}.{func}")));
        }
    }
    let path = path.replace("::", ".");
    let (self_node, func) = scope(doc, here);

    // A local variable or parameter (the first word of a path, Ex. "p" in "p.len()")
    if !path.contains('.') {
        if let Some(func) = func {
            let declared = locals(doc, func, (position.line as usize, end)).into_iter().rev().find(|(name, ..)| name == &path);
            if let Some((name, line, col)) = declared {
                return Some(Target::Local { name, line, col });
            }
        }
    }

    // A path to a field, function, or object
    let mut segments: Vec<&str> = path.split('.').collect();
    let name = segments.pop()?;
    let relative = matches!(segments.first().copied().unwrap_or(name), "self" | "super");
    let start_node = if relative { self_node.clone() } else { None };
    let owner = if segments.is_empty() { None } else { SPath::node(graph, segments.join(".").as_str(), start_node.clone()) };
    let mut found: Option<Target> = None;
    if let Some(owner) = owner {
        let mut owners = vec![owner.clone()];
        owners.extend(Prototype::prototype_nodes(graph, &owner, true)); // functions from types
        for owner in owners {
            if let Some(data) = Func::func(graph, &owner, &name.into()).or_else(|| Field::direct_field(graph, &owner, name)) {
                found = def_for_data(doc, &data);
                if found.is_some() { break; }
            }
        }
    }
    if found.is_none() {
        if let Some(node) = SPath::node(graph, path.as_str(), start_node) {
            found = def_for_node(doc, &node);
        }
    }

    // A type name (Ex. <User>, new User {}, x: User, as Geo.Point)
    if found.is_none() && !relative {
        let ty = match path.rsplit_once('.') {
            Some((qualifier, name)) => graph.find_qualified_type(qualifier, name, self_node.clone()),
            None => graph.find_type(&path, self_node.clone()),
        };
        if let Some(ty) = ty { found = def_for_node(doc, &ty); }
    }

    // A bare standard library call (Ex. pln(..))
    if found.is_none() && !path.contains('.') && line[end..].trim_start().starts_with('(') {
        if graph.libfuncs.get("Std").map(|funcs| funcs.contains_key(path.as_str())).unwrap_or(false) {
            return Some(Target::Lib(format!("Std.{path}")));
        }
    }
    found
}

fn def_for_data(doc: &Doc, data: &DataRef) -> Option<Target> {
    if let Some(index) = doc.info.defs.iter().position(|def| def.data.as_ref() == Some(data)) {
        return Some(Target::Def(index));
    }
    // a field made for an object (Ex. lazily): find the object
    let obj = doc.graph.get_stof_data::<Field>(data).and_then(|field| field.value.try_obj())?;
    def_for_node(doc, &obj)
}

fn def_for_node(doc: &Doc, node: &NodeRef) -> Option<Target> {
    doc.info.defs.iter().position(|def| def.kind != DefKind::Import && def.obj.as_ref() == Some(node)).map(Target::Def)
}


/// Go to definition.
fn definition(docs: &HashMap<Url, Doc>, params: &TextDocumentPositionParams) -> Option<Location> {
    let doc = docs.get(&params.text_document.uri)?;
    let index = match target(doc, &params.position)? {
        Target::Def(index) => index,
        Target::Local { name, line, col } => return Some(Location::new(doc.uri.clone(), word_range(&doc.text, line, col, &name))),
        Target::Lib(_) => return None,
    };
    let def = &doc.info.defs[index];

    if def.kind == DefKind::Import {
        let file = def.target.as_ref()?;
        return Some(Location::new(Url::from_file_path(file).ok()?, Range::default()));
    }
    let file = def.start.file.as_ref()?.to_string();
    let text = match docs.values().find(|doc| doc.path == file) {
        Some(open) => open.text.clone(),
        None => std::fs::read_to_string(&file).ok()?,
    };
    Some(Location::new(Url::from_file_path(&file).ok()?, name_range(&text, def)))
}


/// Hover: signature, type, and docs.
fn hover(docs: &HashMap<Url, Doc>, params: &TextDocumentPositionParams) -> Option<Hover> {
    let doc = docs.get(&params.text_document.uri)?;
    let graph = &doc.graph;
    let markdown = match target(doc, &params.position)? {
        Target::Lib(path) => {
            let (lib, name) = path.split_once('.')?;
            graph.libfuncs.get(lib)?.get(name)?.docs.clone()
        },
        Target::Local { line, .. } => {
            // the line it's declared on (Ex. "let p = new Point { x: 3 };")
            let mut code = doc.text.lines().nth(line)?.trim().to_string();
            if code.chars().count() > 100 { code = code.chars().take(97).collect::<String>() + "..."; }
            format!("```stof
{code}
```")
        },
        Target::Def(index) => {
            let def = &doc.info.defs[index];
            let code = match def.kind {
                DefKind::Import => format!("import '{}'", def.target.as_deref().unwrap_or(def.name.as_str())),
                DefKind::Root => format!("root {}", def.name),
                DefKind::Func => {
                    let func = def.data.as_ref().and_then(|data| graph.get_stof_data::<Func>(data))?;
                    func_signature(graph, func, Some(&def.name))
                },
                DefKind::Field => {
                    let field = def.data.as_ref().and_then(|data| graph.get_stof_data::<Field>(data));
                    match (&def.obj, field) {
                        (Some(obj), _) if is_type(graph, obj) => format!("type {}", def.name),
                        (_, Some(field)) => {
                            let value = field.value.val.read();
                            let mut preview = match value.deref() {
                                _ if def.obj.is_some() => "{ .. }".to_string(),
                                Val::Str(text) => format!("'{text}'"),
                                value => value.print(graph),
                            };
                            if preview.chars().count() > 80 { preview = preview.chars().take(77).collect::<String>() + "..."; }
                            format!("{} {}: {preview}", field_type(graph, field), def.name)
                        },
                        _ => def.name.clone(),
                    }
                },
            };
            let mut markdown = format!("```stof\n{code}\n```");
            if !def.docs.is_empty() { markdown.push_str(&format!("\n\n{}", def.docs)); }
            markdown
        },
    };
    Some(Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: markdown }), range: None })
}


/// Completion: members after "self." (or any path), library functions after "Lib." or "Lib::", types
/// after '<', "new", or "as", and otherwise variables, library functions, libraries, roots, types, and keywords.
fn completion(docs: &HashMap<Url, Doc>, params: &TextDocumentPositionParams) -> Option<Vec<CompletionItem>> {
    let doc = docs.get(&params.text_document.uri)?;
    let graph = &doc.graph;
    let line = doc.text.lines().nth(params.position.line as usize).unwrap_or("").trim_end_matches('\r');
    let cursor = byte_index(line, params.position.character);
    let word_start = line[..cursor].rfind(|c: char| !is_word(c)).map(|index| index + 1).unwrap_or(0);
    let before = &line[..word_start];
    let (self_node, func) = scope(doc, (params.position.line + 1, cursor as u32 + 1));
    let mut items = Vec::new();
    let mut add = |label: &str, kind: CompletionItemKind, detail: Option<String>, docs: &str| {
        if items.iter().any(|item: &CompletionItem| item.label == label) { return; }
        let documentation = (!docs.is_empty()).then(|| Documentation::MarkupContent(MarkupContent { kind: MarkupKind::Markdown, value: docs.to_string() }));
        items.push(CompletionItem { label: label.to_string(), kind: Some(kind), detail, documentation, ..Default::default() });
    };

    // After a path: "self.", "self.config.", "Num.", "Num::"
    if let Some(owner) = before.strip_suffix("::").or_else(|| before.strip_suffix('.')) {
        let owner_start = owner.rfind(|c: char| !(is_word(c) || c == '.' || c == ':')).map(|index| index + 1).unwrap_or(0);
        let owner = owner[owner_start..].replace("::", ".");
        if let Some(funcs) = graph.libfuncs.get(owner.as_str()) {
            if before.ends_with("::") || graph.find_root_named(owner.as_str()).is_none() {
                let mut names: Vec<&String> = funcs.keys().collect();
                names.sort();
                for name in names { add(name, CompletionItemKind::FUNCTION, None, &funcs[name].docs); }
                return Some(items);
            }
        }
        let relative = matches!(owner.split('.').next(), Some("self" | "super"));
        let node = SPath::node(graph, owner.as_str(), if relative { self_node } else { None });
        if let Some(node) = node {
            let mut owners = vec![node.clone()];
            owners.extend(Prototype::prototype_nodes(graph, &node, true)); // functions from types
            for (index, owner) in owners.iter().enumerate() {
                let Some(owner) = owner.node(graph) else { continue; };
                for (name, data) in &owner.data {
                    if !name.chars().all(is_word) { continue; } // can't follow a '.' (Ex. "full name")
                    let docs = doc.info.defs.iter().find(|def| def.data.as_ref() == Some(data)).map(|def| def.docs.as_str()).unwrap_or("");
                    if let Some(func) = graph.get_stof_data::<Func>(data) {
                        add(name, CompletionItemKind::METHOD, Some(func_signature(graph, func, Some(name))), docs);
                    } else if let (0, Some(field)) = (index, graph.get_stof_data::<Field>(data)) {
                        add(name, CompletionItemKind::FIELD, Some(field_type(graph, field)), docs);
                    }
                }
                if index == 0 {
                    for child in &owner.children {
                        if let Some(child) = child.node(graph).filter(|child| child.is_field()) {
                            add(child.name.as_ref(), CompletionItemKind::FIELD, Some("obj".into()), "");
                        }
                    }
                }
            }
        }
        return Some(items);
    }

    // Types
    let mut types: Vec<&String> = graph.typemap.keys().collect();
    types.sort();
    for name in types { add(name, CompletionItemKind::CLASS, Some("type".into()), ""); }
    let trimmed = before.trim_end();
    if trimmed.ends_with('<') || trimmed.ends_with(" new") || trimmed.ends_with(" as") || trimmed.ends_with("->") || trimmed == "new" {
        return Some(items);
    }

    // Variables in scope, then the standard library, libraries, roots, and keywords
    if let Some(func) = func {
        for (name, ..) in locals(doc, func, (params.position.line as usize, word_start)).into_iter().rev() {
            add(&name, CompletionItemKind::VARIABLE, None, "");
        }
    }
    if let Some(std) = graph.libfuncs.get("Std") {
        let mut names: Vec<&String> = std.keys().collect();
        names.sort();
        for name in names { add(name, CompletionItemKind::FUNCTION, None, &std[name].docs); }
    }
    let mut libs: Vec<&str> = graph.libfuncs.keys().map(|lib| lib.as_str()).filter(|lib| *lib != "Std").collect();
    libs.sort();
    for lib in libs { add(lib, CompletionItemKind::MODULE, Some("library".into()), ""); }
    for root in &graph.roots {
        if let Some(name) = root.node_name(graph) { add(name.as_ref(), CompletionItemKind::MODULE, Some("root".into()), ""); }
    }
    for keyword in ["self", "super", "let", "const", "using", "fn", "async", "await", "return", "if", "else", "while", "for", "in", "loop", "break", "continue", "try", "catch", "switch", "new", "as", "true", "false", "null", "import", "root"] {
        add(keyword, CompletionItemKind::KEYWORD, None, "");
    }
    Some(items)
}


/// References: every place in the open documents (and the definition) that resolves to the same thing.
fn references(docs: &HashMap<Url, Doc>, params: &TextDocumentPositionParams, include_declaration: bool) -> Option<Vec<Location>> {
    let doc = docs.get(&params.text_document.uri)?;
    let mut locations = Vec::new();
    match target(doc, &params.position)? {
        Target::Lib(_) => return None,
        Target::Local { name, line, col } => {
            // only in the function that declares it
            let (_, func) = scope(doc, (line as u32 + 1, col as u32 + 1));
            let func = func?;
            let last = func.end.as_ref().map(|end| end.line as usize).unwrap_or(line + 1);
            for (index, text) in doc.text.lines().enumerate().take(last).skip(line) {
                for (start, word) in words(text.trim_end_matches('\r')) {
                    if word != name || in_comment_or_string(text, start) || (!include_declaration && (index, start) == (line, col)) { continue; }
                    let position = Position::new(index as u32, byte_utf16(text, start as u32 + 1));
                    if let Some(Target::Local { line: l, col: c, .. }) = target(doc, &position) {
                        if (l, c) == (line, col) { locations.push(Location::new(doc.uri.clone(), word_range(&doc.text, index, start, &name))); }
                    }
                }
            }
        },
        Target::Def(index) => {
            let def = &doc.info.defs[index];
            if def.kind == DefKind::Import { return None; }
            let key = (def.start.file.clone(), def.start.line, def.start.col);
            let file = def.start.file.as_ref()?.to_string();

            // the definition itself (its file might not be open)
            if include_declaration {
                let location = match docs.values().find(|open| open.path == file) {
                    Some(open) => Location::new(open.uri.clone(), name_range(&open.text, def)),
                    None => Location::new(Url::from_file_path(&file).ok()?, name_range(&std::fs::read_to_string(&file).ok()?, def)),
                };
                locations.push(location);
            }
            for open in docs.values() {
                for (line, text) in open.text.lines().enumerate() {
                    for (start, word) in words(text.trim_end_matches('\r')) {
                        if word != def.name || in_comment_or_string(text, start) { continue; }
                        let position = Position::new(line as u32, byte_utf16(text, start as u32 + 1));
                        let Some(Target::Def(found)) = target(open, &position) else { continue; };
                        let found = &open.info.defs[found];
                        if (found.start.file.clone(), found.start.line, found.start.col) != key { continue; }
                        if open.path == file && (line as u32 + 1, start as u32 + 1) == (def.start.line, def.start.col) { continue; } // added above
                        locations.push(Location::new(open.uri.clone(), word_range(&open.text, line, start, word)));
                    }
                }
            }
        },
    }
    Some(locations)
}


/// Rename a field, function, type, root, or local variable everywhere it's referenced (see `references`).
fn rename(docs: &HashMap<Url, Doc>, params: &TextDocumentPositionParams, new_name: &str) -> Option<WorkspaceEdit> {
    if new_name.is_empty() || !new_name.chars().all(is_word) || new_name.starts_with(|c: char| c.is_ascii_digit()) { return None; }
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for location in references(docs, params, true)? {
        let mut range = location.range;
        if range.end.character - range.start.character > 1 && is_quoted(docs, &location) {
            range.start.character += 1; // a quoted name: keep the quotes
            range.end.character -= 1;
        }
        changes.entry(location.uri).or_default().push(TextEdit::new(range, new_name.to_string()));
    }
    Some(WorkspaceEdit { changes: Some(changes), ..Default::default() })
}

fn is_quoted(docs: &HashMap<Url, Doc>, location: &Location) -> bool {
    let text = match docs.get(&location.uri) {
        Some(doc) => doc.text.clone(),
        None => location.uri.to_file_path().ok().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default(),
    };
    let line = text.lines().nth(location.range.start.line as usize).unwrap_or("");
    line[byte_index(line, location.range.start.character)..].starts_with(['"', '\''])
}


/// Is a byte column of a line in a comment or a string (roughly: one line at a time, and lines starting
/// with '*' are block comments)? Expressions in template strings (`${..}`) are code.
fn in_comment_or_string(line: &str, col: usize) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('*') || trimmed.starts_with("/*") { return true; }
    let mut quote: Option<char> = None; // string being read
    let mut template = false; // in a template string
    let mut expr = false; // in an expression of that template string
    let mut previous = ' ';
    for (index, c) in line.char_indices() {
        if index >= col { break; }
        match quote {
            Some(q) if c == q && previous != '\\' => quote = None,
            Some(_) => {},
            None if template && !expr && c == '`' => template = false,
            None if template && !expr && c == '{' && previous == '$' => expr = true,
            None if template && !expr => {},
            None if expr && c == '}' => expr = false,
            None if !expr && c == '`' => template = true,
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '/' && previous == '/' => return true,
            None => {},
        }
        previous = c;
    }
    quote.is_some() || (template && !expr)
}


/// Range of a word at a 0-based line and byte column.
fn word_range(text: &str, line: usize, col: usize, word: &str) -> Range {
    let start = byte_utf16(line_of(text, line as u32 + 1), col as u32 + 1);
    Range::new(Position::new(line as u32, start), Position::new(line as u32, start + word.encode_utf16().count() as u32))
}


/// Text of a 1-based line (without the line ending).
fn line_of(text: &str, line: u32) -> &str {
    text.lines().nth((line as usize).saturating_sub(1)).unwrap_or("").trim_end_matches('\r')
}

/// UTF-16 column (LSP) of a 1-based byte column.
fn byte_utf16(line: &str, col: u32) -> u32 {
    let mut index = ((col as usize).saturating_sub(1)).min(line.len());
    while !line.is_char_boundary(index) { index -= 1; }
    line[..index].encode_utf16().count() as u32
}

/// UTF-16 column (LSP) of a 1-based char column.
fn char_utf16(line: &str, col: u32) -> u32 {
    line.chars().take((col as usize).saturating_sub(1)).map(|c| c.len_utf16() as u32).sum()
}

/// Byte index in a line of a UTF-16 column (LSP).
fn byte_index(line: &str, character: u32) -> usize {
    let mut units = 0;
    for (index, c) in line.char_indices() {
        if units >= character { return index; }
        units += c.len_utf16() as u32;
    }
    line.len()
}
