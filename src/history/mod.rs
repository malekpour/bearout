// SPDX-License-Identifier: Apache-2.0

//! Repository history and commit policy: exact Git commit facts for a
//! range or a pending commit, exposed to repository-owned history checks.
//! **Experimental.**

// The runner that consumes the capture lands with the history view.
#![allow(dead_code)]

pub mod capture;
