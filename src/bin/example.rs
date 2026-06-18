use std::fmt::Debug;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use futures::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite};
use tracing_subscriber::EnvFilter;

use overseer_core::{
    Component, Daemon, ErrorResponse, Payload, PredefinedCode, ResponseError, ResponseStream,
    ServiceComponent, StatusCode, Streaming, component, handlers, service,
};
use overseer_transport::{
    Flags, TcpTransport, WireMessage, WireOutcome, WireRequest,
    protocol::codec::{read_message, write_message},
};

#[cfg(unix)]
use overseer_transport::UnixTransport;

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
struct PingRequest;

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
    config: Arc<GreetConfig>,
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

#[handlers]
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
}

// A second impl block contributing to the *same* service — ping, greet, and
// test all roll up under "Greeter".
#[handlers]
impl Greeter {
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
/// macro infers each kind from the signature: a `Streaming<T>` parameter means
/// streamed input, a `ResponseStream<T>` return means streamed output.
#[service(id = "echo", version = "0.1")]
struct Echo;

#[handlers]
impl Echo {
    /// Server streaming: one request `n`, a descending stream `n-1 ..= 0`.
    #[rpc]
    async fn countdown(Payload(n): Payload<u32>) -> ResponseStream<u32> {
        ResponseStream::new(futures::stream::iter((0..n).rev().map(Ok)))
    }

    /// Client streaming: a stream of numbers in, their sum out.
    #[rpc]
    async fn sum(mut input: Streaming<u32>) -> overseer_core::Result<u32> {
        let mut total = 0;

        while let Some(item) = input.next().await {
            total += item?;
        }

        Ok(total)
    }

    /// Bidirectional: each inbound line echoed back uppercased.
    #[rpc]
    async fn echo(input: Streaming<String>) -> ResponseStream<String> {
        ResponseStream::new(input.map(|item| item.map(|line| line.to_uppercase())))
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
// Client helpers — all reuse the caller's open stream/socket
// ---------------------------------------------------------------------------

async fn call_stream<S, Req, Resp>(stream: &mut S, id: u64, path: &str, req: &Req) -> Resp
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    Req: Serialize,
    Resp: for<'de> Deserialize<'de>,
{
    let payload = postcard::to_allocvec(req).unwrap();
    let msg = WireMessage::Request(WireRequest {
        id,
        path: path.to_string(),
        payload,
        streaming_input: false,
    });

    write_message(stream, &msg).await.expect("send request");

    let resp = read_message(stream).await.expect("recv response");

    unpack(resp)
}

fn unpack<Resp: for<'de> Deserialize<'de>>(msg: WireMessage) -> Resp {
    match msg {
        WireMessage::Response(r) => match r.outcome {
            WireOutcome::Ok(bytes) => postcard::from_bytes(&bytes).expect("deserialize response"),
            WireOutcome::Err { code, body } => {
                panic!("RPC error: {} ({} body bytes)", describe(code), body.len())
            }
        },
        _ => panic!("unexpected message type"),
    }
}

/// Like `unpack`, but surfaces an error response's `{ code, body }` to the caller
/// instead of panicking, so the client can classify the failure.
fn unpack_result(msg: WireMessage) -> Result<Vec<u8>, (StatusCode, Vec<u8>)> {
    match msg {
        WireMessage::Response(r) => match r.outcome {
            WireOutcome::Ok(bytes) => Ok(bytes),
            WireOutcome::Err { code, body } => Err((code, body)),
        },
        _ => panic!("unexpected message type"),
    }
}

/// Renders a status code's three sections for display.
fn describe(code: StatusCode) -> String {
    format!(
        "predefined={} custom={} retryable={}",
        code.predefined().to_byte(),
        code.custom(),
        code.contains(Flags::RETRYABLE),
    )
}

/// Writes one wire frame, panicking on transport error (example-grade).
async fn write_frame<S>(stream: &mut S, msg: &WireMessage)
where
    S: AsyncWrite + Unpin,
{
    write_message(stream, msg).await.expect("send frame");
}

/// Server streaming: one request, then collect items until end/error.
async fn call_server_stream<S, Req, Resp>(
    stream: &mut S,
    id: u64,
    path: &str,
    req: &Req,
) -> Vec<Resp>
where
    S: AsyncRead + AsyncWrite + Unpin,
    Req: Serialize,
    Resp: DeserializeOwned + Debug,
{
    let payload = postcard::to_allocvec(req).unwrap();
    let open = WireMessage::Request(WireRequest {
        id,
        path: path.to_string(),
        payload,
        streaming_input: false,
    });

    write_frame(stream, &open).await;

    let mut items = Vec::new();

    loop {
        match read_message(stream).await.expect("recv frame") {
            WireMessage::StreamItem { payload, .. } => {
                let data = postcard::from_bytes(&payload).expect("decode item");
                items.push(data);
            }
            WireMessage::StreamEnd { .. } => break,
            WireMessage::StreamError { code, .. } => {
                eprintln!("stream error: {}", describe(code));
                break;
            }
            _ => panic!("unexpected frame in server stream"),
        }
    }

    items
}

/// Client streaming: open with `streaming_input`, send items, half-close, then
/// read the single response.
async fn call_client_stream<S, Req, Resp>(
    stream: &mut S,
    id: u64,
    path: &str,
    items: &[Req],
) -> Resp
where
    S: AsyncRead + AsyncWrite + Unpin,
    Req: Serialize,
    Resp: for<'de> Deserialize<'de>,
{
    let open = WireMessage::Request(WireRequest {
        id,
        path: path.to_string(),
        payload: Vec::new(),
        streaming_input: true,
    });

    write_frame(stream, &open).await;

    for item in items {
        let payload = postcard::to_allocvec(item).unwrap();

        write_frame(stream, &WireMessage::StreamItem { id, payload }).await;
    }

    write_frame(stream, &WireMessage::StreamEnd { id }).await;

    unpack(read_message(stream).await.expect("recv response"))
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Runs the scripted demo: each unary call, then one of each streaming kind.
async fn exercise<S>(stream: &mut S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ping: PingResponse = call_stream(stream, 1, "Greeter.ping", &PingRequest).await;
    println!("ping             →  {}", ping.message);

    let greet: GreetResponse = call_stream(
        stream,
        2,
        "Greeter.greet",
        &GreetRequest {
            name: "World".to_string(),
        },
    )
    .await;
    println!("greet            →  {}", greet.message);

    // Trigger the classified error: an empty name returns a GreetError carrying a
    // predefined category, a custom subcode, the RETRYABLE flag, and a body.
    let payload = postcard::to_allocvec(&GreetRequest {
        name: String::new(),
    })
    .unwrap();
    let req = WireMessage::Request(WireRequest {
        id: 6,
        path: "Greeter.greet".to_string(),
        payload,
        streaming_input: false,
    });

    write_frame(stream, &req).await;

    match unpack_result(read_message(stream).await.expect("recv response")) {
        Ok(_) => println!("greet (empty)    →  unexpected success"),
        Err((code, body)) => {
            let detail: GreetErrorBody = postcard::from_bytes(&body).expect("decode error body");

            println!(
                "greet (empty)    →  error: {} reason={:?}",
                describe(code),
                detail.reason,
            );
        }
    }

    let countdown: Vec<u32> = call_server_stream(stream, 3, "Echo.countdown", &50u32).await;
    println!("countdown (srv)  →  {countdown:?}");

    let total: u32 = call_client_stream(stream, 4, "Echo.sum", &[1u32, 2, 3, 4]).await;
    println!("sum (client)     →  {total}");

    // Scripted bidi: send a few words, read each echoed back uppercased.
    let id = 5;
    let open = WireMessage::Request(WireRequest {
        id,
        path: "Echo.echo".to_string(),
        payload: Vec::new(),
        streaming_input: true,
    });

    write_frame(stream, &open).await;

    for word in ["hello", "stream", "world"] {
        let payload = postcard::to_allocvec(&word.to_string()).unwrap();

        write_frame(stream, &WireMessage::StreamItem { id, payload }).await;

        match read_message(stream).await.expect("recv echo") {
            WireMessage::StreamItem { payload, .. } => {
                let echoed: String = postcard::from_bytes(&payload).unwrap();
                println!("echo (bidi)      →  {echoed}");
            }
            _ => panic!("unexpected frame in bidi echo"),
        }
    }

    write_frame(stream, &WireMessage::StreamEnd { id }).await;
    let _ = read_message(stream).await; // server's StreamEnd
}

/// Interactive bidi: stream stdin lines to `Echo.echo`, print each reply.
async fn interactive_echo<S>(stream: &mut S, id: u64)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let open = WireMessage::Request(WireRequest {
        id,
        path: "Echo.echo".to_string(),
        payload: Vec::new(),
        streaming_input: true,
    });

    write_frame(stream, &open).await;

    println!("interactive echo — type lines, Ctrl-D to end:");

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    while let Some(line) = lines.next_line().await.expect("read stdin") {
        let payload = postcard::to_allocvec(&line).unwrap();

        write_frame(stream, &WireMessage::StreamItem { id, payload }).await;

        match read_message(stream).await.expect("recv echo") {
            WireMessage::StreamItem { payload, .. } => {
                let echoed: String = postcard::from_bytes(&payload).unwrap();
                println!("  ← {echoed}");
            }
            WireMessage::StreamError { code, .. } => {
                eprintln!("stream error: {}", describe(code));
                return;
            }
            WireMessage::StreamEnd { .. } => break,
            _ => panic!("unexpected frame in echo"),
        }
    }

    write_frame(stream, &WireMessage::StreamEnd { id }).await;
    let _ = read_message(stream).await; // server's StreamEnd

    println!("[echo stream closed]");
}

async fn run_client(transport: TransportKind, interactive: bool) {
    match transport {
        TransportKind::Tcp => {
            use tokio::net::TcpStream;

            let mut stream = TcpStream::connect(TCP_ADDR).await.expect("connect TCP");
            println!("[connected → {TCP_ADDR}]");

            drive(&mut stream, interactive).await;

            drop(stream);
            println!("[connection closed]");
        }

        #[cfg(unix)]
        TransportKind::Unix => {
            use tokio::net::UnixStream;

            let mut stream = UnixStream::connect(UNIX_SOCK).await.expect("connect Unix");
            println!("[connected → {UNIX_SOCK}]");

            drive(&mut stream, interactive).await;

            drop(stream);
            println!("[connection closed]");
        }
    }
}

/// Dispatches to the interactive echo loop or the scripted demo.
async fn drive<S>(stream: &mut S, interactive: bool)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if interactive {
        interactive_echo(stream, 1).await;
    } else {
        exercise(stream).await;
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
            run_client(transport, false).await;
            Ok(())
        }
        Command::Echo { transport } => {
            run_client(transport, true).await;
            Ok(())
        }
    }
}
