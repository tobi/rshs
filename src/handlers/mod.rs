//! Request handlers for HTTP (`GET`/`HEAD`/`PUT`/`DELETE`/`OPTIONS`) and WebDAV
//! (`PROPFIND`/`MKCOL`/`COPY`/`MOVE`/`PROPPATCH`/`LOCK`/`UNLOCK`) methods.

pub mod http;
pub mod locks;
pub mod webdav;
