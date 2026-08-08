use std::io::Read as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use semver::Version;
use thiserror::Error;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, Trap};

use super::{ComponentRenderer, RendererCommand};

const RENDERER_ABI_VERSION: Version = Version::new(0, 1, 0);
const EPOCH_TICK: Duration = Duration::from_millis(10);

wasmtime::component::bindgen!({
    path: "wit/renderer.wit",
    world: "renderer",
});

/// Hard resource limits applied to component loading and each render call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentLimits {
    /// Largest portable component accepted from disk.
    pub component_bytes: usize,
    /// Largest canonical request body transferred into a component.
    pub input_bytes: usize,
    /// Maximum stable resource IDs supplied to one render call.
    pub resources: usize,
    /// Maximum bytes in one resource ID or renderer request metadata field.
    pub metadata_bytes: usize,
    /// Largest response body accepted from a component.
    pub output_bytes: usize,
    /// Maximum instrumented Wasm fuel consumed by instantiation and rendering.
    pub fuel: u64,
    /// Wall-clock execution deadline enforced through Wasmtime epochs.
    pub deadline: Duration,
    /// Maximum bytes per guest linear memory.
    pub memory_bytes: usize,
    /// Maximum number of guest linear memories.
    pub memories: usize,
    /// Maximum elements in each guest table.
    pub table_elements: usize,
    /// Maximum number of guest tables.
    pub tables: usize,
    /// Maximum number of component and core instances.
    pub instances: usize,
}

impl Default for ComponentLimits {
    fn default() -> Self {
        Self {
            component_bytes: 4 * 1024 * 1024,
            input_bytes: 8 * 1024 * 1024,
            resources: 100_000,
            metadata_bytes: 4 * 1024,
            output_bytes: 4 * 1024 * 1024,
            fuel: 10_000_000,
            deadline: Duration::from_secs(2),
            memory_bytes: 32 * 1024 * 1024,
            memories: 4,
            table_elements: 10_000,
            tables: 4,
            instances: 32,
        }
    }
}

/// Shared capability-free Wasmtime engine for presentation components.
pub struct ComponentRendererHost {
    engine: Engine,
    limits: ComponentLimits,
    stop_epoch: Arc<AtomicBool>,
    epoch_thread: Option<std::thread::JoinHandle<()>>,
}

/// Host-validated canonical request supplied to one presentation component.
#[derive(Clone, Copy, Debug)]
pub struct ComponentRenderRequest<'a> {
    pub command: RendererCommand,
    pub format: &'a str,
    pub media_type: &'a str,
    pub tooling_schema: &'a Version,
    pub resources: &'a [String],
    pub color: bool,
    pub payload: &'a [u8],
}

impl ComponentRendererHost {
    /// Creates a component host with fuel and epoch interruption enabled.
    pub fn new(limits: ComponentLimits) -> Result<Self, ComponentRenderError> {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .epoch_interruption(true)
            .max_wasm_stack(512 * 1024);
        let engine = Engine::new(&config).map_err(ComponentRenderError::Engine)?;
        let ticker = engine.clone();
        let stop_epoch = Arc::new(AtomicBool::new(false));
        let ticker_stop = Arc::clone(&stop_epoch);

        let epoch_thread = std::thread::Builder::new()
            .name(String::from("upwell-renderer-epoch"))
            .spawn(move || {
                while !ticker_stop.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    ticker.increment_epoch();
                }
            })
            .map_err(ComponentRenderError::EpochThread)?;

        Ok(Self {
            engine,
            limits,
            stop_epoch,
            epoch_thread: Some(epoch_thread),
        })
    }

    /// Loads, validates, instantiates, and invokes one explicitly configured component.
    pub fn render(
        &self,
        renderer: &ComponentRenderer,
        request: ComponentRenderRequest<'_>,
    ) -> Result<Vec<u8>, ComponentRenderError> {
        if !renderer.abi().matches(&RENDERER_ABI_VERSION) {
            return Err(ComponentRenderError::Abi {
                required: renderer.abi().clone(),
                host: RENDERER_ABI_VERSION.clone(),
            });
        }
        if !renderer.tooling_schema().matches(request.tooling_schema) {
            return Err(ComponentRenderError::ToolingSchema {
                required: renderer.tooling_schema().clone(),
                document: request.tooling_schema.clone(),
            });
        }
        let input_size = request_size(request, self.limits)?;
        if input_size > self.limits.input_bytes {
            return Err(ComponentRenderError::InputTooLarge {
                size: input_size,
                limit: self.limits.input_bytes,
            });
        }

        let bytes = read_bounded(renderer.path(), self.limits.component_bytes)?;
        let component = Component::from_binary(&self.engine, &bytes)
            .map_err(ComponentRenderError::InvalidComponent)?;
        let component_type = component.component_type();

        if let Some((name, _)) = component_type.imports(&self.engine).next() {
            return Err(ComponentRenderError::HostImport {
                name: name.to_owned(),
            });
        }

        let linker = Linker::<HostState>::new(&self.engine);
        let instance_pre = linker
            .instantiate_pre(&component)
            .map_err(ComponentRenderError::InvalidComponent)?;
        let renderer_pre =
            RendererPre::new(instance_pre).map_err(ComponentRenderError::InvalidComponent)?;
        let mut store = self.store()?;
        let bindings = renderer_pre
            .instantiate(&mut store)
            .map_err(classify_execution_error)?;
        let component_request = RenderRequest {
            abi_version: RENDERER_ABI_VERSION.to_string(),
            command: request.command.as_str().to_owned(),
            format: request.format.to_owned(),
            media_type: request.media_type.to_owned(),
            tooling_schema: request.tooling_schema.to_string(),
            resources: request.resources.to_vec(),
            color: request.color,
            payload: request.payload.to_vec(),
        };
        let output = bindings
            .call_render(&mut store, &component_request)
            .map_err(classify_execution_error)?
            .map_err(sanitize_rejection)?;

        let output_size = response_size(&output, self.limits)?;
        if output_size > self.limits.output_bytes {
            return Err(ComponentRenderError::OutputTooLarge {
                size: output_size,
                limit: self.limits.output_bytes,
            });
        }
        if output.format != request.format {
            return Err(ComponentRenderError::ResponseClaim {
                field: "format",
                expected: request.format.to_owned(),
                found: output.format,
            });
        }
        if output.media_type != request.media_type {
            return Err(ComponentRenderError::ResponseClaim {
                field: "media type",
                expected: request.media_type.to_owned(),
                found: output.media_type,
            });
        }
        let selected = request
            .resources
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if output
            .resources
            .iter()
            .any(|resource| !selected.contains(resource.as_str()))
        {
            return Err(ComponentRenderError::UnselectedResource);
        }

        if renderer.utf8() {
            std::str::from_utf8(&output.body).map_err(ComponentRenderError::InvalidUtf8)?;
        }

        Ok(output.body)
    }

    fn store(&self) -> Result<Store<HostState>, ComponentRenderError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .memories(self.limits.memories)
            .table_elements(self.limits.table_elements)
            .tables(self.limits.tables)
            .instances(self.limits.instances)
            .build();
        let mut store = Store::new(&self.engine, HostState { limits });

        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(ComponentRenderError::Engine)?;
        store.epoch_deadline_trap();
        store.set_epoch_deadline(deadline_ticks(self.limits.deadline));
        // Keep canonical-ABI lifting bounded independently of the stricter accepted body size.
        // The exact output contract is checked after lifting; guest memory limits cap this transfer.
        store.set_hostcall_fuel(self.limits.memory_bytes);

        Ok(store)
    }
}

impl Drop for ComponentRendererHost {
    fn drop(&mut self) {
        self.stop_epoch.store(true, Ordering::Relaxed);
        if let Some(thread) = self.epoch_thread.take() {
            let _ = thread.join();
        }
    }
}

struct HostState {
    limits: StoreLimits,
}

/// Component loading, compatibility, sandbox, or execution failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ComponentRenderError {
    #[error("failed to configure the WebAssembly renderer engine")]
    Engine(#[source] anyhow::Error),
    #[error("failed to start the WebAssembly renderer deadline clock")]
    EpochThread(#[source] std::io::Error),
    #[error("failed to read renderer component `{path}`")]
    Read {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("renderer component `{path}` exceeds the {limit}-byte file limit")]
    ComponentTooLarge {
        path: std::path::PathBuf,
        limit: usize,
    },
    #[error("renderer component is invalid or does not export the Upwell renderer world")]
    InvalidComponent(#[source] anyhow::Error),
    #[error("renderer component imports forbidden host capability `{name}`")]
    HostImport { name: String },
    #[error("renderer requires ABI `{required}`, but the host provides `{host}`")]
    Abi {
        required: semver::VersionReq,
        host: Version,
    },
    #[error("renderer requires tooling schema `{required}`, but the input uses `{document}`")]
    ToolingSchema {
        required: semver::VersionReq,
        document: Version,
    },
    #[error("renderer input is {size} bytes, exceeding the {limit}-byte limit")]
    InputTooLarge { size: usize, limit: usize },
    #[error("renderer request has {count} resources, exceeding the {limit}-resource limit")]
    TooManyResources { count: usize, limit: usize },
    #[error("renderer request {field} is {size} bytes, exceeding the {limit}-byte limit")]
    MetadataTooLarge {
        field: &'static str,
        size: usize,
        limit: usize,
    },
    #[error("renderer output is {size} bytes, exceeding the {limit}-byte limit")]
    OutputTooLarge { size: usize, limit: usize },
    #[error("renderer output is not valid UTF-8")]
    InvalidUtf8(#[source] std::str::Utf8Error),
    #[error("renderer rejected the request: {0}")]
    Rejected(String),
    #[error("renderer response changed the {field} claim from `{expected}` to `{found}`")]
    ResponseClaim {
        field: &'static str,
        expected: String,
        found: String,
    },
    #[error("renderer response claims a resource outside the host-selected request")]
    UnselectedResource,
    #[error("renderer exhausted its execution fuel")]
    OutOfFuel,
    #[error("renderer exceeded its execution deadline")]
    Deadline,
    #[error("renderer trapped during execution")]
    Trap(#[source] anyhow::Error),
}

fn read_bounded(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, ComponentRenderError> {
    let file = std::fs::File::open(path).map_err(|source| ComponentRenderError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));

    file.take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| ComponentRenderError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > limit {
        return Err(ComponentRenderError::ComponentTooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }

    Ok(bytes)
}

fn deadline_ticks(deadline: Duration) -> u64 {
    let deadline = deadline.as_nanos();
    let tick = EPOCH_TICK.as_nanos();
    let ticks = deadline.saturating_add(tick.saturating_sub(1)) / tick;

    u64::try_from(ticks.max(1)).unwrap_or(u64::MAX)
}

fn request_size(
    request: ComponentRenderRequest<'_>,
    limits: ComponentLimits,
) -> Result<usize, ComponentRenderError> {
    if request.resources.len() > limits.resources {
        return Err(ComponentRenderError::TooManyResources {
            count: request.resources.len(),
            limit: limits.resources,
        });
    }

    let abi_version = RENDERER_ABI_VERSION.to_string();
    let tooling_schema = request.tooling_schema.to_string();
    let fields = [
        ("ABI version", abi_version.as_str()),
        ("command", request.command.as_str()),
        ("format", request.format),
        ("media type", request.media_type),
        ("tooling schema", tooling_schema.as_str()),
    ];
    let mut size = framed_size(request.payload.len());

    for (field, value) in fields {
        if value.len() > limits.metadata_bytes {
            return Err(ComponentRenderError::MetadataTooLarge {
                field,
                size: value.len(),
                limit: limits.metadata_bytes,
            });
        }
        size = size.saturating_add(framed_size(value.len()));
    }
    size = size.saturating_add(std::mem::size_of::<u32>());
    for resource in request.resources {
        if resource.len() > limits.metadata_bytes {
            return Err(ComponentRenderError::MetadataTooLarge {
                field: "resource ID",
                size: resource.len(),
                limit: limits.metadata_bytes,
            });
        }
        size = size.saturating_add(framed_size(resource.len()));
    }

    Ok(size)
}

fn response_size(
    response: &RenderResponse,
    limits: ComponentLimits,
) -> Result<usize, ComponentRenderError> {
    if response.resources.len() > limits.resources {
        return Err(ComponentRenderError::TooManyResources {
            count: response.resources.len(),
            limit: limits.resources,
        });
    }
    let mut size = framed_size(response.body.len());
    for (field, value) in [
        ("format", &response.format),
        ("media type", &response.media_type),
    ] {
        if value.len() > limits.metadata_bytes {
            return Err(ComponentRenderError::MetadataTooLarge {
                field,
                size: value.len(),
                limit: limits.metadata_bytes,
            });
        }
        size = size.saturating_add(framed_size(value.len()));
    }
    size = size.saturating_add(std::mem::size_of::<u32>());
    for resource in &response.resources {
        if resource.len() > limits.metadata_bytes {
            return Err(ComponentRenderError::MetadataTooLarge {
                field: "resource ID",
                size: resource.len(),
                limit: limits.metadata_bytes,
            });
        }
        size = size.saturating_add(framed_size(resource.len()));
    }

    Ok(size)
}

const fn framed_size(content: usize) -> usize {
    std::mem::size_of::<u32>().saturating_add(content)
}

fn sanitize_rejection(rejection: String) -> ComponentRenderError {
    const LIMIT: usize = 1024;
    let mut sanitized = String::with_capacity(rejection.len().min(LIMIT));

    for character in rejection.chars() {
        let fragment = match character {
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            character if character.is_control() => format!("\\u{{{:x}}}", character as u32),
            character => character.to_string(),
        };
        if sanitized.len().saturating_add(fragment.len()) > LIMIT {
            sanitized.push_str("...");
            break;
        }
        sanitized.push_str(&fragment);
    }

    ComponentRenderError::Rejected(sanitized)
}

fn classify_execution_error(error: anyhow::Error) -> ComponentRenderError {
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => ComponentRenderError::OutOfFuel,
        Some(Trap::Interrupt) => ComponentRenderError::Deadline,
        _ => ComponentRenderError::Trap(error),
    }
}

#[cfg(test)]
mod tests;
