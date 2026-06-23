//! `--stats` / `--discover`: metric aggregation and dashboard rendering.
//!
//! [`agg`] parses `metrics.jsonl` into a [`StatsAgg`]; [`render`] turns that
//! into the terminal dashboard, the JSON view, and `--discover` hints.

mod agg;
mod render;

pub(crate) use agg::*;
pub(crate) use render::*;
