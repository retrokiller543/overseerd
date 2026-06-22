use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use overseer_core::{Component, Daemon, ErrorResponse, Payload, PredefinedCode, ResponseError, ServiceComponent, StatusCode, component, handlers, service};
use overseer_transport::{Flags, TcpTransport};

#[cfg(unix)]
use overseer_transport::UnixTransport;

// The client SDK is generated and compiled only under the `client` feature; the
// daemon build pulls in none of it.
#[cfg(feature = "client")]
use overseer::{ClientConnection, ClientError, ClientTransport};
#[cfg(feature = "client")]
use tokio::io::AsyncBufReadExt;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "example", about = "Overseer ping/greet example")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the daemon on the selected transport.
    Daemon {
        #[arg(value_enum, default_value = "tcp")]
        transport: TransportKind,
    },
    /// Run the scripted client (unary + one of each streaming kind).
    Client {
        #[arg(value_enum, default_value = "tcp")]
        transport: TransportKind,
    },
    /// Open an interactive bidirectional echo stream over stdin.
    Echo {
        #[arg(value_enum, default_value = "tcp")]
        transport: TransportKind,
    },
}

#[derive(Clone, ValueEnum)]
enum TransportKind {
    Tcp,
    #[cfg(unix)]
    Unix,
}

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

const TCP_ADDR: &str = "127.0.0.1:9001";
#[cfg(unix)]
const UNIX_SOCK: &str = "/tmp/overseer-example.sock";

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
struct PingResponse {
    message: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct GreetRequest {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct GreetResponse {
    message: String,
}

/// The application subcode carried in the custom section of a `GreetError`.
const GREET_EMPTY_SUBCODE: u16 = 42;

/// The structured error body a `GreetError` serializes for the client to decode.
#[derive(Serialize, Deserialize, Debug)]
struct GreetErrorBody {
    reason: String,
}

/// A handler error demonstrating a status code: a predefined category, an
/// application-owned custom subcode, the `RETRYABLE` flag, and a structured body.
#[derive(Debug)]
enum GreetError {
    EmptyName,
}

impl std::fmt::Display for GreetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GreetError::EmptyName => write!(f, "name must not be empty"),
        }
    }
}

impl ResponseError for GreetError {
    type Body = GreetErrorBody;
    
    fn status_code(&self) -> StatusCode {
        StatusCode::new_with_custom(
            PredefinedCode::BadInput,
            Flags::empty(),
            GREET_EMPTY_SUBCODE,
        )
    }

    fn error_response(self) -> ErrorResponse {
        let code = self.status_code();
        let body = GreetErrorBody {
            reason: self.to_string(),
        };

        ErrorResponse::with_serialized_body(code, &body)
    }
}

// ---------------------------------------------------------------------------
// Components — the dependency chain the container builds bottom-up:
//   GreetConfig  (manual instance, `with_component`)
//     → Greeting (system-built via `#[component]`, resolves GreetConfig)
//       → Greeter (the service, resolves Greeting)
// ---------------------------------------------------------------------------

/// Raw config, provided as an instance via `with_component`. `#[derive(Component)]`
/// supplies the id/name that registration needs.
#[derive(Component)]
struct GreetConfig {
    greeting: String,
}

/// A system-constructed component: `#[component]` registers a field-injection
/// factory, so the container builds it from its `Arc<T>` dependencies.
#[component]
struct Greeting {
    config: Arc<GreetConfig>
}

impl Greeting {
    fn message(&self, name: &str) -> String {
        format!("{}, {}!", self.config.greeting, name)
    }
}

/// A stateful service. Identity lives on the type via `#[service]`; the
/// singleton holds common deps (`greeting`), resolved from the container.
#[service(id = "greeter", version = "0.1")]
struct Greeter {
    greeting: Arc<Greeting>,
}

// ---------------------------------------------------------------------------
// RPC handlers
//
// `#[handlers]` contributes each `#[rpc]` method to the service of `Self`.
// Several impl blocks may target one service. `&self` methods read the
// singleton's common deps; parameters are extracted by type (`Payload<T>`).
// ---------------------------------------------------------------------------

// `#[handlers(client_trait = GreeterApi)]` additionally generates a
// `GreeterApi<T>` trait (and a `GreeterClient<T>` impl of it) under the `client`
// feature, so callers can depend on the trait and mock it; `#[async_trait]` keeps
// it `dyn`-compatible. It is generic over the transport `T` because streaming
// return types name the transport's stream handle. A service whose RPCs are split
// across several `#[handlers]` blocks may still generate a client, but only a
// single block may carry it (the client struct is defined once).
#[handlers(client_trait = GreeterApi)]
impl Greeter {
    // An explicit `#[init]` constructor; overrides the field-injection default.
    // Its fixed-name `init` marker makes a second `#[init]` a compile error.
    #[init]
    fn init(greeting: Arc<Greeting>) -> Self {
        Self { greeting }
    }

    // No `&self`: `ping` needs no common deps, so it stays a plain associated
    // fn with direct dispatch (no per-call singleton lookup).
    #[rpc]
    async fn ping() -> PingResponse {
        PingResponse {
            message: "pong".to_string(),
        }
    }

    #[rpc]
    async fn greet(
        &self,
        Payload(req): Payload<GreetRequest>,
    ) -> Result<GreetResponse, GreetError> {
        if req.name.is_empty() {
            return Err(GreetError::EmptyName);
        }

        Ok(GreetResponse {
            message: self.greeting.message(&req.name),
        })
    }

    #[rpc]
    async fn test() {}
}

#[derive(Component)]
struct ManualService;

impl ServiceComponent for ManualService {
    const VERSION: Option<&'static str> = None;
}

#[handlers]
impl ManualService {
    #[rpc]
    async fn test() -> String {
        String::from("Hello, world!")
    }
}

/// A stateless service demonstrating the three streaming kinds. The `#[rpc]`
/// macro infers each kind from the signature: a stream-typed parameter (here
/// `impl Stream<Item = T>`) means streamed input, a stream-typed return means
/// streamed output. Handlers return their stream unwrapped — the macro adapts it
/// onto the wire — so a stream straight from an SDK works as-is.
#[service(id = "echo", version = "0.1")]
struct Echo;

#[handlers]
impl Echo {
    /// Server streaming: one request `n`, a descending stream `n-1 ..= 0`. The
    /// items are infallible (`Item = u32`), so no per-item `Result` wrapping.
    #[rpc]
    async fn countdown(Payload(n): Payload<u32>) -> impl Stream<Item = u32> {
        futures::stream::iter((0..n).rev())
    }

    /// Client streaming: a stream of numbers in, their sum out.
    #[rpc]
    async fn sum(input: impl Stream<Item = u32>) -> u32 {
        input.fold(0u32, |total, n| async move { total + n }).await
    }

    /// Bidirectional: each inbound line echoed back uppercased.
    #[rpc]
    async fn echo(input: impl Stream<Item = String>) -> impl Stream<Item = String> {
        input.map(|line| line.to_uppercase())
    }
}

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

async fn run_daemon(transport: TransportKind) -> overseer_core::Result<()> {
    let daemon = Daemon::builder("greeter")
        .auto_discover()
        .with_service(ManualService)
        .with_component(GreetConfig {
            greeting: "Hello".to_string(),
        })
        .build()
        .await?;

    println!("{}", daemon.registry);

    match transport {
        TransportKind::Tcp => {
            let t = TcpTransport::bind(TCP_ADDR).await?;

            daemon.serve(t).await
        }

        #[cfg(unix)]
        TransportKind::Unix => {
            let t = UnixTransport::bind(UNIX_SOCK)?;

            daemon.serve(t).await
        }
    }
}

// ---------------------------------------------------------------------------
// Client
//
// Compiled only under `--features client`. The generated `GreeterClient`/`EchoClient`
// (and the `dyn`-compatible `GreeterApi` trait) talk to the daemon over a
// `ClientConnection`, touching only the shared request/response types — never the
// handler bodies. The `daemon` subcommand builds with none of this present.
// ---------------------------------------------------------------------------

/// Scripted demo: each unary call (including the classified error), then one of
/// each streaming kind. Generic over the transport so it serves TCP and Unix alike.
#[cfg(feature = "client")]
async fn exercise<T: ClientTransport>(greeter: GreeterClient<T>, echo: EchoClient<T>) {
    let ping = greeter.ping().await.expect("ping");
    println!("ping             →  {}", ping.message);

    let greet = greeter
        .greet(&GreetRequest {
            name: "World".to_string(),
        })
        .await
        .expect("greet");
    println!("greet            →  {}", greet.message);

    // Trigger the classified error. `greet` returns `Result<_, GreetError>`, and
    // `GreetError::Body` is `GreetErrorBody`, so the generated client hands back an
    // `ErrorBody<GreetErrorBody>` that decodes directly — the error type and its
    // serialized body need not be the same.
    match greeter
        .greet(&GreetRequest {
            name: String::new(),
        })
        .await
    {
        Ok(_) => println!("greet (empty)    →  unexpected success"),

        Err(ClientError::Remote(err)) => {
            let detail = err.deserialize().expect("decode error body");

            println!(
                "greet (empty)    →  error: status={:?} reason={:?}",
                err.code(),
                detail.reason,
            );
        }

        Err(e) => println!("greet (empty)    →  transport error: {e}"),
    }

    let mut countdown = echo.countdown(&5u32).await.expect("countdown");
    let mut items = Vec::new();

    while let Some(item) = countdown.next().await {
        items.push(item.expect("countdown item"));
    }

    println!("countdown (srv)  →  {items:?}");

    // Client streaming now mirrors the daemon: hand it the input stream, get the
    // one response back.
    let total = echo
        .sum(futures::stream::iter([1u32, 2, 3, 4]))
        .await
        .expect("sum result");
    println!("sum (client)     →  {total}");

    // Bidi mirrors the daemon too: an input stream in, a response stream out,
    // pumped concurrently. Here the input is a fixed list; the interactive demo
    // below feeds a channel instead.
    let mut replies = echo
        .echo(futures::stream::iter(
            ["hello", "stream", "world"].map(String::from),
        ))
        .await
        .expect("echo");

    while let Some(reply) = replies.next().await {
        println!("echo (bidi)      →  {}", reply.expect("echo reply"));
    }
}

/// Interactive bidi: stream stdin lines to `Echo.echo`, print each reply. The
/// input is a channel the stdin loop pushes into (the caller's "sink"); the
/// response stream is read independently — the two run concurrently.
#[cfg(feature = "client")]
async fn interactive_echo<T: ClientTransport>(echo: EchoClient<T>) {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(16);

    // Adapt the channel receiver into the input `Stream` the call consumes.
    let input = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|line| (line, rx))
    });
    let mut replies = echo.echo(input).await.expect("open echo");

    println!("interactive echo — type lines, Ctrl-D to end:");

    // Feed stdin lines into the input stream on a separate task; closing `tx`
    // (on EOF) half-closes the call so the server ends the response stream.
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();

        while let Some(line) = lines.next_line().await.expect("read stdin") {
            if tx.send(line).await.is_err() {
                break;
            }
        }
    });

    while let Some(reply) = replies.next().await {
        match reply {
            Ok(line) => println!("  ← {line}"),

            Err(e) => {
                eprintln!("stream error: {e}");

                return;
            }
        }
    }

    println!("[echo stream closed]");
}

/// Connects fresh clients over the chosen transport, then runs the scripted demo
/// or the interactive echo loop.
#[cfg(feature = "client")]
async fn run_client(transport: TransportKind, interactive: bool) {
    match transport {
        TransportKind::Tcp => {
            let greeter = GreeterClient::new(
                ClientConnection::connect_tcp(TCP_ADDR).await.expect("connect"),
            );
            let echo =
                EchoClient::new(ClientConnection::connect_tcp(TCP_ADDR).await.expect("connect"));

            println!("[connected → {TCP_ADDR}]");

            if interactive {
                interactive_echo(echo).await;
            } else {
                exercise(greeter, echo).await;
            }
        }

        #[cfg(unix)]
        TransportKind::Unix => {
            let greeter = GreeterClient::new(
                ClientConnection::connect_unix(UNIX_SOCK).await.expect("connect"),
            );
            let echo = EchoClient::new(
                ClientConnection::connect_unix(UNIX_SOCK).await.expect("connect"),
            );

            println!("[connected → {UNIX_SOCK}]");

            if interactive {
                interactive_echo(echo).await;
            } else {
                exercise(greeter, echo).await;
            }
        }
    }
}

/// Entry point for the `client`/`echo` subcommands, present in every build so the
/// daemon-only build still parses the CLI; the work itself needs `--features client`.
async fn run_client_command(transport: TransportKind, interactive: bool) {
    #[cfg(feature = "client")]
    run_client(transport, interactive).await;

    #[cfg(not(feature = "client"))]
    {
        let _ = (transport, interactive);

        eprintln!(
            "the client is generated only under `--features client`; rebuild with it to run this command"
        );
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> overseer_core::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with_target(true)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Daemon { transport } => run_daemon(transport).await,
        Command::Client { transport } => {
            run_client_command(transport, false).await;
            Ok(())
        }
        Command::Echo { transport } => {
            run_client_command(transport, true).await;
            Ok(())
        }
    }
}
