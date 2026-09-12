#![forbid(unsafe_code)]

//! Pure layout and CPU rendering for Realm's layer-shell status surfaces.

mod model;
mod render;
mod text;

pub use model::{
    surface_config, Anchor, Damage, Element, ElementRole, Frame, Layer, LogicalRect,
    SurfaceCommand, SurfaceConfig, SurfaceKind, SurfaceLifecycle,
};
pub use render::BarRenderer;
