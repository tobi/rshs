//! Internal utilities — error handling (`OrStatus` trait), path resolution with
//! traversal guards, HTTP-date time formatting, and batch filesystem metadata
//! operations.

pub(crate) mod error;
pub(crate) mod path;
pub(crate) mod scandir;
pub(crate) mod time;
