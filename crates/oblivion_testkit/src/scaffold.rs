//! The captured startup graph as inspectable metadata: not an executable x86 ABI or a linear startup runner.
//!
//! Original addresses are identifiers only. Dispositions are copied from the C++ repository and record the **C++**
//! port: `ImplementedBehavior` means that port has the body. Use [`RUST_PORTED_THROUGH`] for what this repository has.

use crate::scaffold_data::{DEPENDENCIES, FUNCTIONS};

/// IDA: N/A the last startup phase whose code this repository has, kept by hand at each milestone.
pub const RUST_PORTED_THROUGH: &str =
    "Phases 00 to 22 (cutover 2026-11-06); see CLAUDE.md, Current state";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    PendingBody,
    UnresolvedIndirect,
    ImplementedLocal,
    ObsoleteRuntime,
    HostSupplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeDisposition {
    Pending,
    Unresolved,
    ObsoleteRuntime,
    HostSupplied,
    ImplementedBehavior,
}

/// What a stub answers. The C++ scaffold defines a method per address so the host signature cannot impersonate the
/// original ABI; a record makes the same answer, with nothing to call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingCall {
    pub identity: u32,
    pub resolution: Resolution,
}

#[derive(Clone, Copy, Debug)]
pub struct Dependency {
    pub identity: u32,
    pub name: &'static str,
    pub original_signature: &'static str,
    pub body_captured: bool,
    pub disposition: ScopeDisposition,
    pub stub: PendingCall,
}

#[derive(Clone, Copy, Debug)]
pub struct CallSite {
    pub site: u32,
    /// Zero means unresolved; the identity then uses `site`.
    pub target: u32,
    pub pointer_slot: u32,
    pub instruction: &'static str,
    pub disposition: ScopeDisposition,
    pub stub: PendingCall,
}

#[derive(Clone, Copy, Debug)]
pub struct BasicBlock {
    pub start: u32,
    pub end: u32,
    pub tail_instruction: &'static str,
    pub successors: &'static [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct FunctionPlan {
    pub entry: u32,
    pub name: &'static str,
    pub calls: &'static [CallSite],
    pub blocks: &'static [BasicBlock],
}

/// IDA: N/A manifest access. 100% metadata; 0% original execution.
#[must_use]
pub const fn functions() -> &'static [FunctionPlan] {
    FUNCTIONS
}

/// IDA: N/A manifest access. 100% metadata; 0% original execution.
#[must_use]
pub const fn dependencies() -> &'static [Dependency] {
    DEPENDENCIES
}

/// IDA: N/A lookup. `None` means an uncaptured body.
#[must_use]
pub fn find(entry: u32) -> Option<&'static FunctionPlan> {
    FUNCTIONS.iter().find(|function| function.entry == entry)
}
