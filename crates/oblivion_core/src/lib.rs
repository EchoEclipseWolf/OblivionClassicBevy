//! Original behaviour that came from the MSVC CRT, the x87 FPU or small shared routines.
//! Everything order- or bit-sensitive in the port goes through here rather than local copies.

pub mod crt;
pub mod error;
pub mod fixtures;
pub mod hash;
pub mod host_path;
pub mod logging;
pub mod pointer_list;
pub mod profile;
pub mod reader;
pub mod sort;
pub mod x87;
