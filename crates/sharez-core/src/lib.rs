//! Platform-independent types shared by every ShareZ crate: geometry, captured frames and the
//! description of the running desktop session.

pub mod frame;
pub mod geom;
pub mod session;

pub use frame::Frame;
pub use geom::{Mapping, PixelRect, Rect};
pub use session::{Desktop, DisplayServer, Session};
