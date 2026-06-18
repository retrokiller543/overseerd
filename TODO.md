1. [x] Stream support — all four kinds (unary, server/client/bidi) at the runtime, transport, and extractor level, with the `#[rpc]` macro inferring `OperationKind` from the signature.
2. [x] Remove the hard requirement for the #[rpc] macro on functions to return Result<T, E>, instead we should migrate to a trait of `Responder` that can be implemented for different return types, this allows for empty responses, results, optionals, infallible, etc all in the type system like normal rust! this will also require a trait for error responses aswell to ensure they also can be returned via the transport.
3. [ ] Generate client sdk from descriptors, we need to investigate how this should be done since it can either be a runtime thing or a complete crate we generate.
4. [ ] Arc<dyn T>, Vec<Arc<dyn T>>, HashMap<String, Arc<dyn T>> support for dependencies. This is a common pattern for dependency injection, and we should support it out of the box. this includes support for provide = dyn T and provide = [dyn T, dyn Y] on #[component] and #[service], and also adding qualifiers and primary attrs to help usability of injecting deps.
5. [ ] Support custom protocols (what is sent over the wire, hard to do but can be powerful)
6. [ ] Multi-frame messages, see large payloads and handle reading it up to a configurable max frame size or infinity (should be configurable by the user)
7. [ ] Config system built in for the daemon with optional hot reload support. Must be typed and preferably be able to specify a way to read from env in the config struct like `${ENV_VAR}` where it can be replaced when reading it in. Take spring as inspiration for this. One cool addtion to this would be the ability to script the config, so configs like if this value is set then x, y and z happens aswell and gets configured.
8. [ ] Native daemon support, i.e. integrate with things like systemd on linux and launchd on macos to send ready signals, handle shutdown signals, and generally be a good citizen on the platform. This would be one of the biggest selling points of this outside of the DI complexity being managed for you.
9. [ ] Health checks to systemd, launchd, etc that can be based on user defined health checks in the code.
10. [ ] Feature gate things like inventory for platforms or devs that dont want DI to happen via auto-discovery, this would disable just that flow and optimize generated code for manual registration instead.
11. [ ] Add quic transport.
12. [ ] Add h3 transport.
13. [ ] Add TLS support for transports.
14. [ ] Add status code support for responses, mainly error responses but potentially also for successful responses
