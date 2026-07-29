use cargo_overseerd::{GraphDirection, GraphRelationFamily};
use clap::ValueEnum;

/// Supported check and doctor output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ReportFormat {
    /// Human-readable terminal output.
    #[default]
    Terminal,
    /// Versioned machine-readable JSON.
    Json,
}

/// Supported inspection output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum InspectFormat {
    /// Human-readable generic inspection.
    #[default]
    Text,
    /// Canonical tooling document JSON.
    Json,
}

/// Canonical export payload.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ExportFormat {
    /// Successful canonical tooling document.
    #[default]
    Document,
    /// Complete canonical success or failure probe envelope.
    Envelope,
}

/// Supported graph output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum GraphFormat {
    /// Human-readable terminal list.
    #[default]
    Text,
    /// Mermaid flowchart source.
    Mermaid,
    /// Graphviz DOT source.
    Dot,
    /// Canonical graph-view JSON.
    Json,
}

/// Supported explanation output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ExplainFormat {
    /// Human-readable terminal explanation.
    #[default]
    Text,
    /// Canonical resource-explanation JSON.
    Json,
}

/// Semantic graph relationship family accepted by the CLI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum GraphFamily {
    #[default]
    All,
    Dependencies,
    Composition,
    Scopes,
    Lifecycle,
    Ownership,
}

impl From<GraphFamily> for GraphRelationFamily {
    fn from(family: GraphFamily) -> Self {
        match family {
            GraphFamily::All => Self::All,
            GraphFamily::Dependencies => Self::Dependencies,
            GraphFamily::Composition => Self::Composition,
            GraphFamily::Scopes => Self::Scopes,
            GraphFamily::Lifecycle => Self::Lifecycle,
            GraphFamily::Ownership => Self::Ownership,
        }
    }
}

/// Semantic graph traversal direction accepted by the CLI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum GraphTraversalDirection {
    #[default]
    Both,
    Upstream,
    Downstream,
}

impl From<GraphTraversalDirection> for GraphDirection {
    fn from(direction: GraphTraversalDirection) -> Self {
        match direction {
            GraphTraversalDirection::Both => Self::Both,
            GraphTraversalDirection::Upstream => Self::Upstream,
            GraphTraversalDirection::Downstream => Self::Downstream,
        }
    }
}

/// Automatic, forced, or disabled terminal behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum TerminalPolicy {
    /// Enable behavior only for an interactive terminal.
    #[default]
    Auto,
    /// Always enable behavior.
    Always,
    /// Never enable behavior.
    Never,
}

/// Generic resource kind accepted by inspection filters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum InspectResourceKind {
    Application,
    Protocol,
    Plugin,
    Component,
    Provider,
    ConfigBinding,
    Hook,
    Lifecycle,
    Scope,
    Type,
    Contribution,
    Contributor,
    PluginSlot,
}

/// CLI provider kind accepted by inspection filters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum InspectCliProviderKind {
    Args,
    Command,
    CommandSet,
}
