//! zilog silicon - shared SDL2/wgpu/egui frontend, seeded from bytebox's own
//! frontend (see TODO.txt in trust-80 for the extraction plan). For now this
//! only exposes what's already been decoupled from bytebox-core: the wgpu
//! rendering pipeline. The bin target (`main.rs`) still builds the full
//! bytebox-specific frontend directly from its own `mod` declarations,
//! unaffected by this library target - the two don't share compiled code
//! yet, just this same source tree.

pub mod console_log;
pub mod egui_gpu;
pub mod renderer;
pub mod status_panel;
pub mod ui_scale;
