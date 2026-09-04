// SPDX-License-Identifier: Apache-2.0

//! The contained module loader. `load()` paths resolve only beneath the
//! rules root: they must be relative, normalized, end in `.star`, and never
//! pass through a symbolic link. Modules are loaded depth-first, linted,
//! typechecked, evaluated once under the resource limits, frozen, and cached
//! by normalized path. Cycles, missing modules, and escapes are reported with
//! the import chain.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::values::Registry;
use starlark::analysis::{AstModuleLint, EvalSeverity};
use starlark::any::AnyLifetime;
use starlark::environment::{FrozenModule, Globals, Module};
use starlark::eval::{Evaluator, ReturnFileLoader};
use starlark::syntax::{AstModule, Dialect};
use starlark::typing::{AstModuleTypecheck, Interface};

use super::Printer;
use super::limits::apply_limits;
use crate::bootstrap::Limits;
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

/// The Starlark dialect Bearout evaluates: starlark-rust's extended dialect,
/// which adds type annotations and f-strings to the standard language.
pub const DIALECT: Dialect = Dialect::Extended;

/// Loaded modules keyed by normalized project path.
#[derive(Default)]
pub struct Modules {
    cache: HashMap<String, FrozenModule>,
    interfaces: HashMap<String, Interface>,
}

/// Everything a module evaluation needs from the host.
pub struct Loader<'h> {
    pub tree: &'h dyn ReadTree,
    pub rules_root: &'h ProjectPath,
    /// Globals for loaded modules.
    pub library: &'h Globals,
    pub limits: Limits,
    pub cancel: Arc<AtomicBool>,
}

/// A parsed module together with its resolved dependencies.
pub struct Prepared {
    pub path: ProjectPath,
    text: String,
    ast: AstModule,
    interface_inputs: HashMap<String, Interface>,
    loads: HashMap<String, ProjectPath>,
}

/// Resolve a `load()` string to a project path beneath the rules root.
pub fn resolve(rules_root: &ProjectPath, module_id: &str) -> Result<ProjectPath, String> {
    if module_id.starts_with('/') || module_id.starts_with('\\') {
        return Err(format!(
            "`{module_id}` is absolute; load paths are relative to the rules root"
        ));
    }
    let relative = ProjectPath::parse(module_id).map_err(|error| format!("load path {error}"))?;
    if relative.as_str().is_empty() {
        return Err("load path must not be empty".to_owned());
    }
    if relative.extension() != Some("star") {
        return Err(format!("`{module_id}` must be a `.star` module"));
    }
    if relative.as_str() != module_id {
        return Err(format!(
            "`{module_id}` must be written in normalized form `{relative}`"
        ));
    }
    Ok(rules_root.join(&relative))
}

impl Loader<'_> {
    /// Load `path` and everything it loads, transitively, as a library
    /// module. Diagnostics are appended; `Err` means the module is unusable.
    pub fn load(
        &self,
        modules: &mut Modules,
        path: &ProjectPath,
        chain: &mut Vec<String>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), ()> {
        if modules.cache.contains_key(path.as_str()) {
            return Ok(());
        }
        let prepared = self.prepare(modules, path, chain, diagnostics)?;
        let frozen = self.evaluate(modules, prepared, self.library, None, diagnostics)?;
        modules.cache.insert(path.as_str().to_owned(), frozen);
        Ok(())
    }

    /// Read, parse, load dependencies, lint, and typecheck one module.
    pub fn prepare(
        &self,
        modules: &mut Modules,
        path: &ProjectPath,
        chain: &mut Vec<String>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<Prepared, ()> {
        let key = path.as_str().to_owned();
        if let Some(start) = chain.iter().position(|entry| *entry == key) {
            let cycle: Vec<&str> = chain[start..]
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(key.as_str()))
                .collect();
            diagnostics.push(Diagnostic::new(
                Code::ScriptLoad,
                path.as_str(),
                format!("import cycle: {}", cycle.join(" -> ")),
            ));
            return Err(());
        }
        chain.push(key);
        let result = self.prepare_inner(modules, path, chain, diagnostics);
        chain.pop();
        result
    }

    fn prepare_inner(
        &self,
        modules: &mut Modules,
        path: &ProjectPath,
        chain: &mut Vec<String>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<Prepared, ()> {
        let via = import_chain(chain);
        let report = |message: String| {
            Diagnostic::new(Code::ScriptLoad, path.as_str(), format!("{message}{via}"))
        };

        match self.tree.symlink_component(path) {
            Ok(None) => {}
            Ok(Some(link)) => {
                diagnostics.push(report(format!(
                    "`{link}` is a symbolic link; modules must not be reached through links"
                )));
                return Err(());
            }
            Err(error) => {
                diagnostics.push(report(format!("cannot inspect module path: {error}")));
                return Err(());
            }
        }
        let text = match self.tree.read_text(path) {
            Ok(text) => text,
            Err(error) => {
                diagnostics.push(report(format!("cannot read module: {error}")));
                return Err(());
            }
        };
        let parse = || {
            AstModule::parse(path.as_str(), text.clone(), &DIALECT).map_err(|error| {
                report(format!(
                    "cannot parse module: {}",
                    error.without_diagnostic()
                ))
                .at_line(error_line(&error))
            })
        };
        let ast = match parse() {
            Ok(ast) => ast,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                return Err(());
            }
        };

        let mut loads: HashMap<String, ProjectPath> = HashMap::new();
        for load in ast.loads() {
            let line = u32::try_from(load.span.resolve_span().begin.line + 1).ok();
            let resolved = match resolve(self.rules_root, load.module_id) {
                Ok(resolved) => resolved,
                Err(error) => {
                    diagnostics.push(report(error).at_line(line));
                    return Err(());
                }
            };
            if self.load(modules, &resolved, chain, diagnostics).is_err() {
                diagnostics.push(report(format!("cannot load `{}`", load.module_id)).at_line(line));
                return Err(());
            }
            loads.insert(load.module_id.to_owned(), resolved);
        }

        for lint in ast.lint(None) {
            if matches!(lint.severity, EvalSeverity::Error | EvalSeverity::Warning) {
                diagnostics.push(
                    Diagnostic::new(
                        Code::ScriptLint,
                        path.as_str(),
                        format!("{}: {}", lint.short_name, lint.problem),
                    )
                    .at_line(u32::try_from(lint.location.resolve_span().begin.line + 1).ok()),
                );
            }
        }

        let interface_inputs: HashMap<String, Interface> = loads
            .iter()
            .filter_map(|(id, resolved)| {
                modules
                    .interfaces
                    .get(resolved.as_str())
                    .map(|interface| (id.clone(), interface.clone()))
            })
            .collect();
        Ok(Prepared {
            path: path.clone(),
            text,
            ast,
            interface_inputs,
            loads,
        })
    }

    /// Typecheck and evaluate a prepared module under the limits, then freeze it.
    pub fn evaluate(
        &self,
        modules: &mut Modules,
        prepared: Prepared,
        globals: &Globals,
        extra: Option<&Registry>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<FrozenModule, ()> {
        let path = prepared.path;
        let report = |message: String| Diagnostic::new(Code::ScriptLoad, path.as_str(), message);

        let Ok(typecheck_ast) = AstModule::parse(path.as_str(), prepared.text.clone(), &DIALECT)
        else {
            unreachable!("module parsed once already")
        };
        let (errors, _, interface, _) =
            typecheck_ast.typecheck(globals, &prepared.interface_inputs);
        if !errors.is_empty() {
            for error in errors {
                diagnostics.push(
                    report(format!("type error: {}", error.without_diagnostic()))
                        .at_line(error_line(&error)),
                );
            }
            return Err(());
        }

        let resolved_modules: HashMap<&str, &FrozenModule> = prepared
            .loads
            .iter()
            .map(|(id, resolved)| (id.as_str(), &modules.cache[resolved.as_str()]))
            .collect();
        let file_loader = ReturnFileLoader {
            modules: &resolved_modules,
        };
        let printer = Printer::default();
        let ast = prepared.ast;
        let frozen = Module::with_temp_heap(|module| {
            let outcome = {
                let mut eval = Evaluator::new(&module);
                eval.set_loader(&file_loader);
                eval.set_print_handler(&printer);
                eval.enable_static_typechecking(true);
                eval.extra = extra.map(|registry| registry as &dyn AnyLifetime);
                if let Err(error) = apply_limits(&mut eval, self.limits, &self.cancel) {
                    return Err(report(format!("cannot apply limits: {error}")));
                }
                eval.eval_module(ast, globals).map(|_| ())
            };
            match outcome {
                Ok(()) => module
                    .freeze()
                    .map_err(|error| report(format!("cannot freeze module: {error:?}"))),
                Err(error) => Err(Diagnostic::new(
                    Code::ScriptFailure,
                    path.as_str(),
                    format!("module failed: {}", error.without_diagnostic()),
                )
                .at_line(error_line(&error))),
            }
        });
        diagnostics.extend(printer.drain().into_iter().map(|line| {
            Diagnostic::new(
                Code::ScriptOutput,
                path.as_str(),
                format!("module printed: {line}"),
            )
        }));
        let frozen = match frozen {
            Ok(frozen) => frozen,
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                return Err(());
            }
        };
        modules
            .interfaces
            .insert(path.as_str().to_owned(), interface);
        Ok(frozen)
    }
}

fn import_chain(chain: &[String]) -> String {
    if chain.len() > 1 {
        format!(" (imported via {})", chain[..chain.len() - 1].join(" -> "))
    } else {
        String::new()
    }
}

/// One-based line of a Starlark error, when it carries a span.
pub fn error_line(error: &starlark::Error) -> Option<u32> {
    error
        .span()
        .and_then(|span| u32::try_from(span.resolve_span().begin.line + 1).ok())
}
