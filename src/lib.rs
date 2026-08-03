//! Library surface of l0-compressor.
//!
//! The CLI in `src/main.rs` remains the primary product and is unaffected by this file: it
//! keeps its own module tree. What this adds is the ability for another Rust program to reuse
//! the filters *in process*, without shelling out to the binary — which matters when the
//! caller is compressing many small buffers per second rather than wrapping one command.
//!
//! The first consumer is FreeCode's `freecode-compress`, which had grown its own build-log
//! noise reduction (collapse repeated lines, keep head/tail, never drop an error). That is the
//! same job these filters already do, better and with 73 tests behind them. Rather than keep
//! two implementations drifting apart, FreeCode now feeds its build output through
//! [`filter::FilterPipeline`] and applies its own token-budget fitting to what comes out:
//! l0-compressor decides *what is noise*, FreeCode decides *what fits*.
//!
//! Practical consequence, and the reason this exists: every compile log FreeCode shows a model
//! exercises this code. A filtering bug found there is a bug fixed here, for the CLI too.
//!
//! Only [`filter`] is exported. `args`, `config`, `runner`, `recovery`, `telemetry` and `ui`
//! are CLI plumbing with no meaning outside the binary, and exporting them would freeze
//! internals as public API for no gain.

pub mod filter;
