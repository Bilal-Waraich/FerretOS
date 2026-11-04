//! OML-generated task schema, instance bindings, and precomputed CCG constants.
//!
//! `task_schema.rs`, `demo_tasks.rs` and `pipeline_tasks.rs` are raw OML
//! transpiler output, and
//! `ccg_constants.rs` is emitted by `build.rs`; all three are regenerated on
//! every build where OML is available and committed so the kernel builds
//! without it.  `bridge` is hand-written and never overwritten.
//!
//! The OML output is included verbatim inside wrapper modules so that the
//! imports and lint allowances it needs live here rather than being patched
//! into generated text.  `empty_line_after_outer_attr` is allowed because OML
//! emits a file-level `#[allow(dead_code)]` followed by a blank line.

pub mod bridge;
pub mod ccg_constants;

/// Task schema generated from `tasks/task.oml`.
#[allow(dead_code, clippy::upper_case_acronyms, clippy::empty_line_after_outer_attr)]
pub mod task_schema {
    include!("task_schema.rs");
}

/// CA-PIP H/M/L demo instances generated from `tasks/demo_tasks.oml`.
#[allow(dead_code, clippy::empty_line_after_outer_attr)]
pub mod demo_tasks {
    use super::task_schema::TaskConfig;
    include!("demo_tasks.rs");
}

/// Sensor-fusion pipeline instances generated from `tasks/pipeline_tasks.oml`.
#[allow(dead_code, clippy::empty_line_after_outer_attr)]
pub mod pipeline_tasks {
    use super::task_schema::TaskConfig;
    include!("pipeline_tasks.rs");
}
