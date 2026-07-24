use syn::{Block, Expr, Ident, LitStr, Path, Type, Visibility};

/// Parsed input accepted by `app!`.
pub(crate) enum AppInput {
    /// A reusable named application definition.
    Named(NamedApp),
    /// The temporary expression-oriented application builder form.
    Legacy(AppAssembly),
}

/// A reusable named application definition.
pub(crate) struct NamedApp {
    pub(super) visibility: Visibility,
    pub(super) ident: Ident,
    pub(super) assembly: AppAssembly,
}

/// The protocol-specific builder assembly shared by both macro forms.
pub(crate) struct AppAssembly {
    pub(super) name: Expr,
    pub(super) protocol: Type,
    pub(super) services: Vec<Type>,
    pub(super) components: Vec<Expr>,
    pub(super) configs: Vec<ConfigEntry>,
    pub(super) config_manager: Option<ManagerSource<ConfigSettings>>,
    pub(super) directories_manager: Option<ManagerSource<DirSettings>>,
    pub(super) middleware: Vec<Expr>,
    pub(super) guards: Vec<Expr>,
    pub(super) error_handler: Option<Expr>,
    pub(super) overseerd: Option<Path>,
    pub(super) krate: Option<Path>,
    pub(super) phases: AppPhases,
}

/// Application lifecycle phase definitions.
#[derive(Default)]
pub(super) struct AppPhases {
    pub(super) setup: Option<PhaseInput>,
    pub(super) configure: Option<PhaseInput>,
    pub(super) before_build: Option<PhaseInput>,
    pub(super) after_build: Option<PhaseInput>,
    pub(super) serve: Option<PhaseInput>,
}

/// A lifecycle phase implemented by a function or inline block.
pub(super) enum PhaseInput {
    Path(Path),
    Inline { arguments: Vec<Ident>, body: Block },
}

/// How a manager is supplied in the `managers` block.
#[allow(clippy::large_enum_variant)]
pub(super) enum ManagerSource<S> {
    Instance(Expr),
    Configure(S),
}

/// Settings for a macro-constructed `ConfigManager`.
#[derive(Default)]
pub(super) struct ConfigSettings {
    pub(super) source: Option<Expr>,
    pub(super) profiles: Option<Expr>,
    pub(super) sighup: bool,
    pub(super) watch: bool,
    pub(super) debounce: Option<Expr>,
}

/// Settings for a macro-constructed `DirectoriesManager`.
#[derive(Default)]
pub(super) struct DirSettings {
    pub(super) app: Option<Expr>,
    pub(super) root: Option<Expr>,
}

/// One `configs:` entry containing a type and property path.
pub(super) struct ConfigEntry {
    pub(super) ty: Type,
    pub(super) path: LitStr,
}
