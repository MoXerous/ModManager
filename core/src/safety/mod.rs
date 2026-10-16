//! Safe archive handling: member-name validation, archive inspection, atomic writes.
//!
//! Rule source: `docs/spec/behaviors/archive-safety.md` (from `safe_archive.py`).
//! Archives (modpacks) are untrusted input.

pub mod atomic;
pub mod extract;
pub mod inspect;
pub mod path;
