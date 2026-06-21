# nano-jit

`nano-jit` is the optional per-event backend for validated `nano-spec` IR:

1. generate Rust with `nano_spec::codegen::generate_producer_source`,
2. write a temporary `cdylib` crate,
3. invoke Cargo at runtime,
4. load the resulting dynamic library through the platform dynamic loader,
5. call the exported kernel.

This is not the default execution path. It requires a Rust toolchain at runtime
and pays compile latency before the first event. Use it for arbitrary validated
specs at native speed without a manual rebuild. Use the interpreter when runtime
toolchain freedom matters, and AOT codegen when build-time compilation is fine.

The first slice supports the muon spec. Rust values such as `&nano_core::Event`
do not cross the dynamic-library boundary because Rust's ABI is not stable. The
loaded library exports an `extern "C"` entry point that takes plain inputs:
`nMuon`, `Muon_pt` pointer/length, `Muon_eta` pointer/length, and a `#[repr(C)]`
output row. The dylib reconstructs an internal `Event` and calls the generated
Rust producer on its own side of the boundary.

Runtime tests are intentionally opt-in:

```sh
NANO_RUN_JIT=1 cargo test -p nano-jit --features jit
```

Default workspace tests do not compile or run a JIT kernel.
