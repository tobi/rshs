//! Internal utilities — error handling (`OrStatus` trait), path resolution with
//! traversal guards, HTTP-date time formatting, and batch filesystem metadata
//! operations.

pub(crate) mod error;
pub(crate) mod fs_batch;
pub(crate) mod path;
pub(crate) mod time;
