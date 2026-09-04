// SPDX-License-Identifier: Apache-2.0

//! Repository hygiene and formatting over the files a project selects.
//!
//! The kernel knows selected paths, their bytes, the effective text
//! properties from `.editorconfig` files of the same tree, and formatter
//! assignments; it assigns no meaning to a file name or extension of its
//! own. Selection is explicit: `[hygiene] scope = "repository"` means every
//! file of the project as Git knows it (the captured index or revision
//! tree, or for the working directory the tracked plus untracked,
//! non-ignored files beneath the project root), `scope = "declared"`
//! means only the listed roots and files. Symbolic links are never
//! followed, submodules are never entered, and every list is sorted.
//!
//! Only the candidate is ever selected; the comparison baseline is
//! history and is neither checked nor formatted.

pub mod selection;

pub use selection::{Universe, select};
