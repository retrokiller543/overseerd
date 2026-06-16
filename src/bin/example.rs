use std::{future::Future, pin::Pin};

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use overseer_core::{
    Daemon, OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor,
    RpcResponse, ServiceDescriptor, TypeDescriptor,
};
use overseer_transport::{
    TcpTransport, UdpTransport, WireMessage, WireOutcome, WireRequest,
    protocol::codec::{decode, encode, read_message, write_message},
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
    /// Run the client against the selected transport.
    Client {
        #[arg(value_enum, default_value = "tcp")]
        transport: TransportKind,
    },
}

#[derive(Clone, ValueEnum)]
enum TransportKind {
    Tcp,
    Udp,
    #[cfg(unix)]
    Unix,
}

// ---------------------------------------------------------------------------
// Addresses
// ---------------------------------------------------------------------------

const TCP_ADDR: &str = "127.0.0.1:9001";
const UDP_ADDR: &str = "127.0.0.1:9002";
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

// Stand-in service type — macros will generate this from #[service].
struct GreeterService;

// ---------------------------------------------------------------------------
// RPC handlers
// ---------------------------------------------------------------------------

fn ping_handler(
    _ctx: RpcCallContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<RpcResponse>> + Send>> {
    Box::pin(async {
        let response = PingResponse {
            message: "pong".to_string(),
        };
        let payload = postcard::to_allocvec(&response).unwrap();

        Ok(RpcResponse { payload })
    })
}

fn greet_handler(
    ctx: RpcCallContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<RpcResponse>> + Send>> {
    Box::pin(async move {
        let req: GreetRequest = postcard::from_bytes(&ctx.payload).unwrap();
        let response = GreetResponse {
            message: format!("Hello, {}!", req.name),
        };
        let payload = postcard::to_allocvec(&response).unwrap();

        Ok(RpcResponse { payload })
    })
}

// ---------------------------------------------------------------------------
// Static descriptors
// ---------------------------------------------------------------------------

static GREETER_SERVICE_RPCS: [RpcDescriptor; 2] = [
    RpcDescriptor {
        name: "ping",
        operation: OperationKind::Query,
        parameters: &[],
        output: TypeDescriptor::of::<PingResponse>("PingResponse"),
        handler: ping_handler,
    },
    RpcDescriptor {
        name: "greet",
        operation: OperationKind::Command,
        parameters: &[ParameterDescriptor {
            name: "request",
            kind: ParameterKind::Payload,
            ty: TypeDescriptor::of::<GreetRequest>("GreetRequest"),
        }],
        output: TypeDescriptor::of::<GreetResponse>("GreetResponse"),
        handler: greet_handler,
    },
];

static GREETER_SERVICE: ServiceDescriptor = ServiceDescriptor {
    id: "greeter",
    name: "Greeter",
    ty: TypeDescriptor::of::<GreeterService>("GreeterService"),
    version: Some("0.1"),
    rpcs: &GREETER_SERVICE_RPCS,
};

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

async fn run_daemon(transport: TransportKind) -> overseer_core::Result<()> {
    let daemon = Daemon::builder("greeter")
        .service(&GREETER_SERVICE)
        .build()
        .await?;

    println!("{}", daemon.registry);

    match transport {
        TransportKind::Tcp => {
            let t = TcpTransport::bind(TCP_ADDR).await?;
            println!("Listening on TCP  {TCP_ADDR}  (ctrl-c to stop)\n");
            daemon.serve(t).await
        }

        TransportKind::Udp => {
            let t = UdpTransport::bind(UDP_ADDR).await?;
            println!("Listening on UDP  {UDP_ADDR}  (ctrl-c to stop)\n");
            daemon.serve(t).await
        }

        #[cfg(unix)]
        TransportKind::Unix => {
            let t = UnixTransport::bind(UNIX_SOCK)?;
            println!("Listening on Unix {UNIX_SOCK}  (ctrl-c to stop)\n");
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
    });

    write_message(stream, &msg).await.expect("send request");

    let resp = read_message(stream).await.expect("recv response");

    unpack(resp)
}

async fn call_udp<Req, Resp>(socket: &tokio::net::UdpSocket, id: u64, path: &str, req: &Req) -> Resp
where
    Req: Serialize,
    Resp: for<'de> Deserialize<'de>,
{
    let payload = postcard::to_allocvec(req).unwrap();
    let msg = WireMessage::Request(WireRequest {
        id,
        path: path.to_string(),
        payload,
    });
    let bytes = encode(&msg).expect("encode");

    socket.send(&bytes).await.expect("send UDP datagram");

    let mut buf = vec![0u8; 65507];
    let len = socket.recv(&mut buf).await.expect("recv UDP datagram");
    let resp = decode(&buf[..len]).expect("decode");

    unpack(resp)
}

fn unpack<Resp: for<'de> Deserialize<'de>>(msg: WireMessage) -> Resp {
    match msg {
        WireMessage::Response(r) => match r.outcome {
            WireOutcome::Ok(bytes) => postcard::from_bytes(&bytes).expect("deserialize response"),
            WireOutcome::Err(e) => panic!("RPC error: {e}"),
        },
        _ => panic!("unexpected message type"),
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

async fn run_client(transport: TransportKind) {
    match transport {
        TransportKind::Tcp => {
            use tokio::net::TcpStream;

            println!("--- TCP (persistent connection) ---");

            let mut stream = TcpStream::connect(TCP_ADDR).await.expect("connect TCP");
            println!("[connection opened → {TCP_ADDR}]");

            let r1: PingResponse = call_stream(&mut stream, 1, "Greeter.ping", &PingRequest).await;
            println!("call 1  ping   →  {}", r1.message);

            let r2: GreetResponse =
                call_stream(&mut stream, 2, "Greeter.greet", &GreetRequest { name: "World".to_string() }).await;
            println!("call 2  greet  →  {}", r2.message);

            let r3: GreetResponse =
                call_stream(&mut stream, 3, "Greeter.greet", &GreetRequest { name: "Overseer".to_string() }).await;
            println!("call 3  greet  →  {}", r3.message);

            let r4: PingResponse = call_stream(&mut stream, 4, "Greeter.ping", &PingRequest).await;
            println!("call 4  ping   →  {}", r4.message);

            drop(stream);
            println!("[connection closed]");
        }

        TransportKind::Udp => {
            use tokio::net::UdpSocket;

            // One socket, multiple datagrams. The router on the daemon side
            // groups them into a single UdpConnection by peer address.
            println!("--- UDP (persistent session via router) ---");

            let socket = UdpSocket::bind("0.0.0.0:0").await.expect("bind UDP");
            socket.connect(UDP_ADDR).await.expect("connect UDP");
            println!("[socket bound, peer → {UDP_ADDR}]");

            let r1: PingResponse = call_udp(&socket, 1, "Greeter.ping", &PingRequest).await;
            println!("call 1  ping   →  {}", r1.message);

            let r2: GreetResponse =
                call_udp(&socket, 2, "Greeter.greet", &GreetRequest { name: "World".to_string() }).await;
            println!("call 2  greet  →  {}", r2.message);

            let r3: GreetResponse =
                call_udp(&socket, 3, "Greeter.greet", &GreetRequest { name: "Overseer".to_string() }).await;
            println!("call 3  greet  →  {}", r3.message);

            let r4: PingResponse = call_udp(&socket, 4, "Greeter.ping", &PingRequest).await;
            println!("call 4  ping   →  {}", r4.message);

            drop(socket);
            println!("[session ended — router will evict peer on next datagram]");
        }

        #[cfg(unix)]
        TransportKind::Unix => {
            use tokio::net::UnixStream;

            println!("--- Unix socket (persistent connection) ---");

            let mut stream = UnixStream::connect(UNIX_SOCK).await.expect("connect Unix");
            println!("[connection opened → {UNIX_SOCK}]");

            let r1: PingResponse = call_stream(&mut stream, 1, "Greeter.ping", &PingRequest).await;
            println!("call 1  ping   →  {}", r1.message);

            let r2: GreetResponse =
                call_stream(&mut stream, 2, "Greeter.greet", &GreetRequest { name: "World".to_string() }).await;
            println!("call 2  greet  →  {}", r2.message);

            let r3: GreetResponse =
                call_stream(&mut stream, 3, "Greeter.greet", &GreetRequest { name: "Overseer".to_string() }).await;
            println!("call 3  greet  →  {}", r3.message);

            let r4: PingResponse = call_stream(&mut stream, 4, "Greeter.ping", &PingRequest).await;
            println!("call 4  ping   →  {}", r4.message);

            drop(stream);
            println!("[connection closed]");
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> overseer_core::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with_target(true)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Daemon { transport } => run_daemon(transport).await,
        Command::Client { transport } => {
            run_client(transport).await;
            Ok(())
        }
    }
}
