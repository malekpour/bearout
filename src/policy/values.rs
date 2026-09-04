// SPDX-License-Identifier: Apache-2.0

//! Host values of the Starlark ABI. Repository code constructs findings and
//! outputs through `error()`, `warning()`, and `output()`; every field is
//! checked at construction, so a misspelled keyword or a wrong type fails
//! inside the script with a Starlark error that names the call site.

use std::fmt;

use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::none::{NoneOr, NoneType};

use crate::identity;
use crate::paths::ProjectPath;

/// The host value types. `ProvidesStaticType` is an unsafe trait that
/// starlark's derive macro implements; this module exists so that the
/// `unsafe_code` allowance covers exactly these derive sites and nothing
/// handwritten. It holds type definitions and macro-generated impls only.
mod host_types {
    #![allow(unsafe_code)]
    // The `starlark_value` macro requires the named `'v` lifetime.
    #![allow(clippy::elidable_lifetime_names)]

    use std::cell::RefCell;

    use allocative::Allocative;
    use starlark::any::ProvidesStaticType;
    use starlark::starlark_simple_value;
    use starlark::values::{NoSerialize, StarlarkValue, starlark_value};

    use super::SchemaRegistration;

    /// A finding constructed by `error()` or `warning()`.
    #[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType, NoSerialize, Allocative)]
    pub struct Finding {
        /// `true` for `error()`, `false` for `warning()`.
        pub is_error: bool,
        pub message: String,
        /// Identifier of the resource the finding is about, when given.
        pub resource: Option<String>,
        /// Project-relative path of the schema-less document the finding is
        /// about, when given. Exclusive with `resource`.
        pub path: Option<String>,
        /// One-based line in that resource or document, when given.
        pub line: Option<u32>,
        /// Repository-owned rule identifier, when given.
        pub rule: Option<String>,
    }

    starlark_simple_value!(Finding);

    #[starlark_value(type = "bearout.finding")]
    impl<'v> StarlarkValue<'v> for Finding {}

    /// A generation plan entry constructed by `output()`.
    #[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType, NoSerialize, Allocative)]
    pub struct Output {
        /// Template name relative to the templates root.
        pub template: String,
        /// Normalized output path relative to the project root.
        pub path: String,
        /// Rendering context as canonical JSON text.
        pub context: String,
    }

    starlark_simple_value!(Output);

    #[starlark_value(type = "bearout.output")]
    impl<'v> StarlarkValue<'v> for Output {}

    /// Registrations collected while the entry module runs. Callbacks are
    /// stored as synthetic module variables so they survive freezing.
    #[derive(Debug, Default, ProvidesStaticType)]
    pub struct Registry {
        pub schemas: RefCell<Vec<SchemaRegistration>>,
        /// `(name, module variable)`.
        pub checks: RefCell<Vec<(String, String)>>,
        /// `(name, module variable)`.
        pub generators: RefCell<Vec<(String, String)>>,
        counter: RefCell<u32>,
    }

    impl Registry {
        pub(super) fn next_slot(&self, prefix: &str) -> String {
            let mut counter = self.counter.borrow_mut();
            *counter += 1;
            format!("__bearout_{prefix}_{}", *counter)
        }
    }
}

pub use host_types::{Finding, Output, Registry};

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.is_error { "error" } else { "warning" };
        write!(f, "{kind}({:?})", self.message)
    }
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "output({:?}, {:?})", self.template, self.path)
    }
}

/// One schema registration made by the entry module.
#[derive(Debug, Clone)]
pub struct SchemaRegistration {
    pub id: String,
    /// Shape file relative to the rules root.
    pub shape: Option<String>,
    /// Module variable holding the validate callback.
    pub validate: Option<String>,
}

fn fail(message: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!(message))
}

fn finding(
    is_error: bool,
    message: &str,
    resource: NoneOr<&str>,
    path: NoneOr<&str>,
    line: NoneOr<i32>,
    code: NoneOr<&str>,
) -> starlark::Result<Finding> {
    if message.trim().is_empty() {
        return Err(fail("finding message must not be empty".to_owned()));
    }
    let resource = match resource {
        NoneOr::None => None,
        NoneOr::Other(id) => {
            identity::check_id(id).map_err(|error| fail(format!("finding resource: {error}")))?;
            Some(id.to_owned())
        }
    };
    let path = match path {
        NoneOr::None => None,
        NoneOr::Other(text) => {
            let parsed =
                ProjectPath::parse(text).map_err(|error| fail(format!("finding path: {error}")))?;
            if parsed.as_str().is_empty() {
                return Err(fail("finding path must not be empty".to_owned()));
            }
            Some(parsed.as_str().to_owned())
        }
    };
    if resource.is_some() && path.is_some() {
        return Err(fail(
            "a finding names either a `resource` or a `path`, not both".to_owned(),
        ));
    }
    let line = match line {
        NoneOr::None => None,
        NoneOr::Other(line) => Some(
            u32::try_from(line)
                .ok()
                .filter(|line| *line > 0)
                .ok_or_else(|| {
                    fail(format!(
                        "finding line must be a positive integer, found {line}"
                    ))
                })?,
        ),
    };
    let rule = match code {
        NoneOr::None => None,
        NoneOr::Other(code) => {
            identity::check_kind(code).map_err(|error| fail(format!("finding code: {error}")))?;
            Some(code.to_owned())
        }
    };
    Ok(Finding {
        is_error,
        message: message.to_owned(),
        resource,
        path,
        line,
        rule,
    })
}

/// Functions available to every module: the finding and output constructors.
#[starlark_module]
pub fn library(builder: &mut GlobalsBuilder) {
    /// Report an error about a resource or a schema-less document.
    /// `resource` defaults to the resource being validated; project checks
    /// must name a `resource` or a document `path`.
    fn error(
        #[starlark(require = pos)] message: &str,
        #[starlark(require = named, default = NoneOr::None)] resource: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] path: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] line: NoneOr<i32>,
        #[starlark(require = named, default = NoneOr::None)] code: NoneOr<&str>,
    ) -> starlark::Result<Finding> {
        finding(true, message, resource, path, line, code)
    }

    /// Report a warning about a resource or a schema-less document.
    fn warning(
        #[starlark(require = pos)] message: &str,
        #[starlark(require = named, default = NoneOr::None)] resource: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] path: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] line: NoneOr<i32>,
        #[starlark(require = named, default = NoneOr::None)] code: NoneOr<&str>,
    ) -> starlark::Result<Finding> {
        finding(false, message, resource, path, line, code)
    }

    /// Plan one generated file: render `template` to `path` with `context`.
    fn output<'v>(
        #[starlark(require = pos)] template: &str,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = named, default = NoneOr::None)] context: NoneOr<Value<'v>>,
    ) -> starlark::Result<Output> {
        let template = ProjectPath::parse(template)
            .map_err(|error| fail(format!("output template: {error}")))?;
        if template.as_str().is_empty() {
            return Err(fail("output template must not be empty".to_owned()));
        }
        let path =
            ProjectPath::parse(path).map_err(|error| fail(format!("output path: {error}")))?;
        if path.as_str().is_empty() {
            return Err(fail("output path must not be empty".to_owned()));
        }
        let context = match context {
            NoneOr::None => serde_json::Value::Object(serde_json::Map::new()),
            NoneOr::Other(value) => {
                let json = value.to_json_value().map_err(|error| {
                    fail(format!("output context must be JSON-compatible: {error}"))
                })?;
                if !json.is_object() {
                    return Err(fail("output context must be a dict".to_owned()));
                }
                json
            }
        };
        Ok(Output {
            template: template.as_str().to_owned(),
            path: path.as_str().to_owned(),
            context: context.to_string(),
        })
    }
}

fn registry<'a>(eval: &Evaluator<'_, 'a, '_>) -> starlark::Result<&'a Registry> {
    eval.extra
        .and_then(|extra| extra.downcast_ref::<Registry>())
        .ok_or_else(|| {
            fail(
                "schema(), check(), and generator() may only be called from the entry module"
                    .to_owned(),
            )
        })
}

fn require_callable(value: Value<'_>, label: &str) -> starlark::Result<()> {
    let kind = value.get_type();
    if kind == "function" {
        Ok(())
    } else {
        Err(fail(format!("{label} must be a function, found {kind}")))
    }
}

/// Functions available only to the entry module: registration.
#[starlark_module]
pub fn registration(builder: &mut GlobalsBuilder) {
    /// Register a schema identifier with an optional shape file and validator.
    fn schema<'v>(
        #[starlark(require = pos)] id: &str,
        #[starlark(require = named, default = NoneOr::None)] shape: NoneOr<&str>,
        #[starlark(require = named, default = NoneOr::None)] validate: NoneOr<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        identity::check_schema_id(id).map_err(fail)?;
        let registry = registry(eval)?;
        if registry
            .schemas
            .borrow()
            .iter()
            .any(|existing| existing.id == id)
        {
            return Err(fail(format!("schema `{id}` is registered twice")));
        }
        let shape = match shape {
            NoneOr::None => None,
            NoneOr::Other(text) => {
                let path = ProjectPath::parse(text)
                    .map_err(|error| fail(format!("schema shape: {error}")))?;
                if path
                    .file_name()
                    .strip_suffix(".schema.toml")
                    .is_none_or(str::is_empty)
                {
                    return Err(fail(format!(
                        "schema shape `{text}` must be a `.schema.toml` file"
                    )));
                }
                Some(path.as_str().to_owned())
            }
        };
        let validate = match validate {
            NoneOr::None => None,
            NoneOr::Other(function) => {
                require_callable(function, "schema validate")?;
                let slot = registry.next_slot("validate");
                eval.module().set(&slot, function);
                Some(slot)
            }
        };
        registry.schemas.borrow_mut().push(SchemaRegistration {
            id: id.to_owned(),
            shape,
            validate,
        });
        Ok(NoneType)
    }

    /// Register a project-level check.
    fn check<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] function: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        identity::check_kind(name).map_err(|error| fail(format!("check name: {error}")))?;
        require_callable(function, "check function")?;
        let registry = registry(eval)?;
        if registry
            .checks
            .borrow()
            .iter()
            .any(|(existing, _)| existing == name)
        {
            return Err(fail(format!("check `{name}` is registered twice")));
        }
        let slot = registry.next_slot("check");
        eval.module().set(&slot, function);
        registry.checks.borrow_mut().push((name.to_owned(), slot));
        Ok(NoneType)
    }

    /// Register a generator.
    fn generator<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] function: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        identity::check_kind(name).map_err(|error| fail(format!("generator name: {error}")))?;
        require_callable(function, "generator function")?;
        let registry = registry(eval)?;
        if registry
            .generators
            .borrow()
            .iter()
            .any(|(existing, _)| existing == name)
        {
            return Err(fail(format!("generator `{name}` is registered twice")));
        }
        let slot = registry.next_slot("generator");
        eval.module().set(&slot, function);
        registry
            .generators
            .borrow_mut()
            .push((name.to_owned(), slot));
        Ok(NoneType)
    }
}
