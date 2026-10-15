//! Prints what region capture would snap to at a screen position (logical pixels):
//!
//!     cargo run -p sharez-windows --example elements -- 400 300

use sharez_core::Session;
use sharez_windows::{Elements, Source};

fn main() {
    let args: Vec<f64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let [x, y] = args[..] else {
        eprintln!("usage: elements X Y");
        std::process::exit(2);
    };
    let session = Session::detect().expect("a display server");
    let source = Source::for_session(&session).expect("window listing on this desktop");
    async_io::block_on(async {
        let windows = source.windows().await.expect("window list");
        let elements = Elements::snapshot().await.expect("accessibility bus");
        println!("{elements:#?}");
        for target in elements.targets(windows) {
            if target.visible.contains(x, y) {
                println!("window {:?} at {:?}", target.window.title, target.visible);
                for r in elements.elements_at(&target, x, y).await {
                    println!("  element at {r:?}");
                }
                break;
            }
        }
    });
}
