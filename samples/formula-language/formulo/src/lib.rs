// SPDX-License-Identifier: Apache-2.0

//! Formulo: a deliberately small spreadsheet-expression language. This shell
//! only mounts the modules that Bearout generates from the resource graph.

#[path = "generated/ast.rs"]
pub mod ast;
#[path = "generated/lexer.rs"]
pub mod lexer;
#[path = "generated/parser.rs"]
pub mod parser;
