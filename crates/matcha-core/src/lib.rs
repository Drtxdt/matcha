//! Domain types shared by Matcha frontends and services.

use std::path::PathBuf;
use uuid::Uuid;

/// Stable identity for an execution context.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ContextId(Uuid);

impl ContextId {
    /// Creates a fresh context identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ContextId {
    fn default() -> Self {
        Self::new()
    }
}

/// The machine on which commands execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionTarget {
    /// A process on the desktop host.
    Local,
    /// A process reached through a saved SSH profile.
    Ssh { profile_id: String },
}

/// Shell metadata needed to start a terminal session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellProfile {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// A named, visibly distinct place where commands execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionContext {
    pub id: ContextId,
    pub name: String,
    pub target: ExecutionTarget,
    pub shell: ShellProfile,
    pub cwd: Option<PathBuf>,
}
