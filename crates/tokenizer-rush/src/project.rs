//! Editor operations spanning a registered set of Rush modules.
use crate::*;
use themoretheless_tokenizer_core::Span;

pub struct ModuleEditorAnalysis<'s> {
    pub module: String,
    pub analysis: EditorAnalysis<'s>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectEdit {
    pub module: String,
    pub span: Span,
    pub replacement: String,
}
/// Module names available after `import`, sorted and deduplicated.
pub fn import_completions(modules: &[(&str, &Program<'_>)], prefix: &str) -> Vec<String> {
    let mut names: Vec<_> = modules
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| n.starts_with(prefix))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    names
}
// One graph walk for the entire project, including imports in dead branches.
fn import_diagnostics(modules: &[(&str, &Program<'_>)]) -> Vec<Vec<Diagnostic>> {
    let mut diagnostics: Vec<Vec<Diagnostic>> = (0..modules.len()).map(|_| Vec::new()).collect();
    let mut registry = std::collections::HashMap::new();
    for (index, (name, program)) in modules.iter().enumerate() {
        if let Some(previous) = registry.insert(*name, index) {
            diagnostics[index].push(Diagnostic::new(
                program.parsed_clone().module.span,
                "duplicate-module",
                "Module name is registered more than once",
            ));
            diagnostics[previous].push(Diagnostic::new(
                modules[previous].1.parsed_clone().module.span,
                "duplicate-module",
                "Module name is registered more than once",
            ));
        }
    }
    let mut state = vec![0u8; modules.len()];
    let mut stack = Vec::new();
    for root in 0..modules.len() {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        stack.push((root, 0usize));
        while let Some((owner, next)) = stack.last_mut() {
            let imports = modules[*owner].1.import_refs();
            let Some(import) = imports.get(*next) else {
                state[*owner] = 2;
                stack.pop();
                continue;
            };
            *next += 1;
            let Some(&target) = registry.get(import.text) else {
                if diagnostics[*owner].len() < InputLimits::conservative().max_diagnostics {
                    diagnostics[*owner].push(Diagnostic::new(
                        import.span,
                        "unknown-module",
                        "Imported module is not registered",
                    ));
                }
                continue;
            };
            match state[target] {
                0 => {
                    state[target] = 1;
                    stack.push((target, 0));
                }
                1 if diagnostics[*owner].len() < InputLimits::conservative().max_diagnostics => {
                    diagnostics[*owner].push(Diagnostic::new(
                        import.span,
                        "cyclic-import",
                        "Cyclic module import",
                    ));
                }
                _ => {}
            }
        }
    }
    diagnostics
}
/// References include their destination module, while byte spans stay document-local.
pub fn analyze_editor_project<'s>(
    modules: &[(&str, &Program<'s>)],
) -> Vec<ModuleEditorAnalysis<'s>> {
    let interfaces = analysis::module_interfaces(modules);
    let mut graph_diagnostics = import_diagnostics(modules);
    modules
        .iter()
        .enumerate()
        .map(|(index, (name, program))| {
            let mut parsed = program.parsed_clone();
            let (references, bindings, member_completions) =
                analysis::editor_module(&mut parsed, interfaces.clone(), name);
            for diagnostic in std::mem::take(&mut graph_diagnostics[index]) {
                parsed.valid = false;
                if parsed.diagnostics.len() < InputLimits::conservative().max_diagnostics
                    && !parsed
                        .diagnostics
                        .iter()
                        .any(|d| d.code == diagnostic.code && d.span == diagnostic.span)
                {
                    parsed.diagnostics.push(diagnostic);
                }
            }
            ModuleEditorAnalysis {
                module: name.to_string(),
                analysis: EditorAnalysis {
                    parsed,
                    references,
                    bindings,
                    member_completions,
                },
            }
        })
        .collect()
}
/// Return checked text edits; no files are written. Reject invalid names, captures and collisions.
/// Only lexically resolved names and nominal fields/variants are renamed; dynamic record keys remain opaque.
pub fn rename_project_symbol(
    modules: &[(&str, &Program<'_>)],
    document: &str,
    offset: usize,
    replacement: &str,
) -> std::result::Result<Vec<ProjectEdit>, String> {
    let probe = format!("let {replacement}=0");
    let parsed = parse(&probe);
    if !parsed.is_valid()
        || parsed.module.items.len() != 1
        || !matches!(&parsed.module.items[0].kind,StmtKind::Declaration {name,..} if name.text==replacement)
    {
        return Err("Replacement must be one Rush identifier".into());
    }
    let project = analyze_editor_project(modules);
    if project.iter().any(|d| !d.analysis.parsed.is_valid()) {
        return Err("Resolve project diagnostics before renaming".into());
    }
    let current = project
        .iter()
        .find(|d| d.module == document)
        .ok_or("Unknown document")?;
    let contains = |span: Span| span.start <= offset && offset < span.end;
    let target = current
        .analysis
        .references
        .iter()
        .find(|r| contains(r.usage) && r.definition.is_some())
        .map(|r| {
            (
                r.definition_module
                    .as_deref()
                    .unwrap_or(document)
                    .to_owned(),
                r.definition.unwrap(),
            )
        })
        .or_else(|| {
            current
                .analysis
                .bindings
                .iter()
                .find(|b| contains(b.definition))
                .map(|b| (document.to_owned(), b.definition))
        })
        .ok_or("No resolved symbol at cursor")?;
    if modules
        .iter()
        .find(|(name, _)| *name == target.0)
        .is_some_and(|(_, p)| p.imports().iter().any(|i| i.span == target.1))
    {
        return Err("Import module names must be changed in the registry as well".into());
    }
    let mut edits = Vec::new();
    for item in &project {
        for reference in &item.analysis.references {
            if reference.definition == Some(target.1)
                && reference
                    .definition_module
                    .as_deref()
                    .unwrap_or(&item.module)
                    == target.0
            {
                edits.push(ProjectEdit {
                    module: item.module.clone(),
                    span: reference.usage,
                    replacement: replacement.into(),
                });
            }
        }
    }
    edits.push(ProjectEdit {
        module: target.0.clone(),
        span: target.1,
        replacement: replacement.into(),
    });
    edits.sort_by_key(|e| (e.module.clone(), e.span.start));
    edits.dedup();
    let mut sources = Vec::new();
    for (name, program) in modules {
        let mut source = program.parsed_clone().source.to_owned();
        for edit in edits.iter().rev().filter(|e| e.module == *name) {
            source.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        }
        sources.push((name.to_string(), source));
    }
    let programs: Vec<_> = sources
        .iter()
        .map(|(_, s)| Program::compile(s).map_err(|e| e.message))
        .collect::<std::result::Result<_, _>>()?;
    let registry: Vec<_> = sources
        .iter()
        .zip(&programs)
        .map(|((name, _), p)| (name.as_str(), p))
        .collect();
    for program in &programs {
        program.validate_modules(&registry).map_err(|e| e.message)?;
    }
    // Re-resolve every edited use so a syntactically valid capture cannot silently change its target.
    let renamed = analyze_editor_project(&registry);
    let shifted = |module: &str, span: Span| {
        let delta: isize = edits
            .iter()
            .filter(|e| e.module == module && e.span.start < span.start)
            .map(|e| e.replacement.len() as isize - (e.span.end - e.span.start) as isize)
            .sum();
        let start = (span.start as isize + delta) as usize;
        Span::new(start, start + replacement.len())
    };
    let definition = shifted(&target.0, target.1);
    for edit in &edits {
        if edit.module == target.0 && edit.span == target.1 {
            continue;
        }
        let usage = shifted(&edit.module, edit.span);
        let doc = renamed.iter().find(|d| d.module == edit.module).unwrap();
        if !doc.analysis.references.iter().any(|r| {
            r.usage == usage
                && r.definition == Some(definition)
                && r.definition_module.as_deref().unwrap_or(&edit.module) == target.0
        }) {
            return Err("Rename would capture a reference or change its target".into());
        }
    }
    Ok(edits)
}
