//! Single source of truth for the §5 instance mode this orchestrator runs in.
//!
//! The MVP instance runs in `user_mode`, which models intrinsic affect and
//! therefore permits intrinsic-affect targets. `organization_mode` and
//! `worker_mode` reject them; both the precondition path (Wiring-A) and the
//! effects path (Wiring-B) validate against this value before touching
//! `UniverseState`. Keep the literal here so the mode can be configured later
//! without scattering `InstanceMode` literals across the services.

use ubu_core::core::InstanceMode;

/// The instance mode for this MVP deployment (`user_mode`).
pub const MVP_INSTANCE_MODE: InstanceMode = InstanceMode::UserMode;
