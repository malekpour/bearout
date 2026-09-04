// SPDX-License-Identifier: Apache-2.0

//! The repository policy runtime: Starlark, loaded through a contained
//! loader, evaluated under resource limits, with a narrow versioned ABI.
//!
//! The entry module registers schemas, checks, and generators. Callbacks
//! receive frozen resource or project views and return lists of host values
//! built with `error()`, `warning()`, and `output()`.

pub mod limits;
pub mod loader;
pub mod values;
pub mod views;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use starlark::PrintHandler;
use starlark::environment::{Globals, GlobalsBuilder, LibraryExtension};
use starlark::eval::Evaluator;
use starlark::values::list::ListRef;
use starlark::values::{OwnedFrozenValue, Value, ValueLike};

use crate::bootstrap::{Bootstrap, Limits};
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;
use loader::{Loader, Modules};
use values::{Finding, Output, Registry};

/// Captures `print()` output so it never reaches the process streams.
#[derive(Default)]
pub struct Printer(RefCell<Vec<String>>);

impl Printer {
    fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

impl PrintHandler for Printer {
    fn println(&self, text: &str) -> starlark::Result<()> {
        self.0.borrow_mut().push(text.to_owned());
        Ok(())
    }
}

/// A registered schema.
pub struct Schema {
    /// Shape file relative to the project root, when declared.
    pub shape: Option<ProjectPath>,
    validate: Option<OwnedFrozenValue>,
}

/// The loaded policy of one project.
pub struct Policy {
    /// Registered schemas by identifier.
    pub schemas: BTreeMap<String, Schema>,
    /// Registered checks in registration order.
    pub checks: Vec<(String, OwnedFrozenValue)>,
    /// Registered generators in registration order.
    pub generators: Vec<(String, OwnedFrozenValue)>,
    /// Registered history checks in registration order. Experimental.
    pub history_checks: Vec<(String, OwnedFrozenValue)>,
    /// The entry module path.
    pub entry: ProjectPath,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    /// Highest tick count observed in any call, for measuring limits.
    pub max_ticks: std::cell::Cell<u64>,
    /// Highest heap allocation observed in any call, for measuring limits.
    pub max_heap_bytes: std::cell::Cell<u64>,
}

/// Why a call into repository code produced no usable result.
pub enum CallError {
    /// The call raised, was cancelled, or exceeded a limit (B013).
    Failure { message: String, line: Option<u32> },
    /// The call returned something the ABI does not accept (B014).
    Result(String),
}

/// What a call produced besides its result.
pub struct CallOutcome<T> {
    pub result: Result<T, CallError>,
    pub printed: Vec<String>,
}

fn library_globals() -> Globals {
    GlobalsBuilder::extended_by(&[
        LibraryExtension::StructType,
        LibraryExtension::RecordType,
        LibraryExtension::EnumType,
        LibraryExtension::Map,
        LibraryExtension::Filter,
        LibraryExtension::Partial,
        LibraryExtension::Print,
        LibraryExtension::Pprint,
        LibraryExtension::Json,
        LibraryExtension::Typing,
    ])
    .with(values::library)
    .build()
}

fn entry_globals() -> Globals {
    GlobalsBuilder::extended_by(&[
        LibraryExtension::StructType,
        LibraryExtension::RecordType,
        LibraryExtension::EnumType,
        LibraryExtension::Map,
        LibraryExtension::Filter,
        LibraryExtension::Partial,
        LibraryExtension::Print,
        LibraryExtension::Pprint,
        LibraryExtension::Json,
        LibraryExtension::Typing,
    ])
    .with(values::library)
    .with(values::registration)
    .build()
}

/// Load the entry module and everything it loads. Returns the policy when
/// the entry evaluated; diagnostics are appended either way.
pub fn load(
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    cancel: Arc<AtomicBool>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Policy> {
    let library = library_globals();
    let entry_globals = entry_globals();
    let loader = Loader {
        tree,
        rules_root: &bootstrap.rules_root,
        library: &library,
        limits: bootstrap.limits,
        cancel: Arc::clone(&cancel),
    };
    let mut modules = Modules::default();
    let mut chain = Vec::new();
    let prepared = loader
        .prepare(&mut modules, &bootstrap.entry, &mut chain, diagnostics)
        .ok()?;
    let registry = Registry::default();
    let frozen = loader
        .evaluate(
            &mut modules,
            prepared,
            &entry_globals,
            Some(&registry),
            diagnostics,
        )
        .ok()?;

    let fetch = |slot: &str| {
        frozen
            .get(slot)
            .expect("registered slot exists in frozen entry module")
    };
    let mut schemas = BTreeMap::new();
    for registration in registry.schemas.borrow().iter() {
        let shape = registration.shape.as_deref().map(|shape| {
            let relative = ProjectPath::parse(shape).expect("validated at registration");
            bootstrap.rules_root.join(&relative)
        });
        schemas.insert(
            registration.id.clone(),
            Schema {
                shape,
                validate: registration.validate.as_deref().map(fetch),
            },
        );
    }
    let checks = registry
        .checks
        .borrow()
        .iter()
        .map(|(name, slot)| (name.clone(), fetch(slot)))
        .collect();
    let generators = registry
        .generators
        .borrow()
        .iter()
        .map(|(name, slot)| (name.clone(), fetch(slot)))
        .collect();
    let history_checks = registry
        .history_checks
        .borrow()
        .iter()
        .map(|(name, slot)| (name.clone(), fetch(slot)))
        .collect();
    Some(Policy {
        schemas,
        checks,
        generators,
        history_checks,
        entry: bootstrap.entry.clone(),
        limits: bootstrap.limits,
        cancel,
        max_ticks: std::cell::Cell::new(0),
        max_heap_bytes: std::cell::Cell::new(0),
    })
}

impl Policy {
    /// Call the validator registered for `schema` with a resource view.
    pub fn validate(
        &self,
        schema: &str,
        resource: &OwnedFrozenValue,
    ) -> Option<CallOutcome<Vec<Finding>>> {
        let callback = self.schemas.get(schema)?.validate.as_ref()?;
        Some(self.call(callback, resource, findings))
    }

    /// Call a check with the project view.
    pub fn check(
        &self,
        callback: &OwnedFrozenValue,
        project: &OwnedFrozenValue,
    ) -> CallOutcome<Vec<Finding>> {
        self.call(callback, project, findings)
    }

    /// Call a history check with the history view.
    pub fn history(
        &self,
        callback: &OwnedFrozenValue,
        history: &OwnedFrozenValue,
    ) -> CallOutcome<Vec<Finding>> {
        self.call(callback, history, findings)
    }

    /// Call a generator with the project view.
    pub fn plan(
        &self,
        callback: &OwnedFrozenValue,
        project: &OwnedFrozenValue,
    ) -> CallOutcome<Vec<Output>> {
        self.call(callback, project, outputs)
    }

    fn call<T>(
        &self,
        callback: &OwnedFrozenValue,
        argument: &OwnedFrozenValue,
        interpret: fn(Value<'_>) -> Result<T, String>,
    ) -> CallOutcome<T> {
        let printer = Printer::default();
        let result = starlark::environment::Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.set_print_handler(&printer);
            eval.enable_static_typechecking(true);
            let result = if let Err(error) = apply(&mut eval, self.limits, &self.cancel) {
                Err(CallError::Failure {
                    message: format!("cannot apply limits: {error}"),
                    line: None,
                })
            } else {
                let function = module.heap().access_owned_frozen_value(callback);
                let view = module.heap().access_owned_frozen_value(argument);
                match eval.eval_function(function, &[view], &[]) {
                    Ok(value) => interpret(value).map_err(CallError::Result),
                    Err(error) => Err(CallError::Failure {
                        message: error.without_diagnostic().to_string(),
                        line: loader::error_line(&error),
                    }),
                }
            };
            self.max_ticks
                .set(self.max_ticks.get().max(eval.get_total_tick_count()));
            let allocated = eval.heap().allocated_bytes() as u64;
            self.max_heap_bytes
                .set(self.max_heap_bytes.get().max(allocated));
            result
        });
        CallOutcome {
            result,
            printed: printer.drain(),
        }
    }
}

fn apply(
    eval: &mut Evaluator<'_, '_, '_>,
    limits: Limits,
    cancel: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    limits::apply_limits(eval, limits, cancel)
}

fn findings(value: Value<'_>) -> Result<Vec<Finding>, String> {
    let list = ListRef::from_value(value)
        .ok_or_else(|| format!("must return a list of findings, found {}", value.get_type()))?;
    list.iter()
        .map(|item| {
            item.downcast_ref::<Finding>().cloned().ok_or_else(|| {
                format!(
                    "list item must be error() or warning(), found {}",
                    item.get_type()
                )
            })
        })
        .collect()
}

fn outputs(value: Value<'_>) -> Result<Vec<Output>, String> {
    let list = ListRef::from_value(value)
        .ok_or_else(|| format!("must return a list of outputs, found {}", value.get_type()))?;
    list.iter()
        .map(|item| {
            item.downcast_ref::<Output>()
                .cloned()
                .ok_or_else(|| format!("list item must be output(), found {}", item.get_type()))
        })
        .collect()
}

/// Attribute a script failure to a module path with a stable code.
pub fn failure_diagnostic(path: &str, label: &str, error: &CallError) -> Diagnostic {
    match error {
        CallError::Failure { message, line } => Diagnostic::new(
            Code::ScriptFailure,
            path,
            format!("{label} failed: {message}"),
        )
        .at_line(*line),
        CallError::Result(message) => {
            Diagnostic::new(Code::ScriptResult, path, format!("{label} {message}"))
        }
    }
}
