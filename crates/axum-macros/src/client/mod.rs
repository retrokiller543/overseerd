//! Building protocol-agnostic client hints for HTTP routes.
//!
//! The framework owns client generation in `macros-core`; this module classifies HTTP handler
//! inputs and supplies the request, response, streaming, and wasm-specific token plans.

mod inputs;
mod path;
mod request;
mod response;
mod streaming;
mod unary;
mod wasm;

pub(crate) use inputs::{Body, BodyKind, classify, collect_wire_types};
pub(crate) use path::{hole_ident, hole_param_types, parse_template};
pub(crate) use response::{is_opaque_response, response_type};
pub(crate) use streaming::{
    ServerWrap, build_client_stream_method, build_stream_client_method, classify_stream_return,
    stream_item,
};
pub(crate) use unary::build_client_method;
pub(crate) use wasm::{WasmBackend, extra_client_tokens, wasm_client_struct};
