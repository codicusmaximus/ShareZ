//! Needs a real X server, so it only runs with `SHAREZ_X11_TESTS=1` (CI provides Xvfb).

use sharez_capture::Backend;

#[test]
fn freezes_the_x11_root_window() {
    if std::env::var_os("SHAREZ_X11_TESTS").is_none() {
        eprintln!("skipped: set SHAREZ_X11_TESTS=1 with an X server on $DISPLAY");
        return;
    }
    let frame = futures_lite::future::block_on(Backend::X11.freeze()).expect("X11 capture");
    assert!(frame.width > 0 && frame.height > 0);
    assert_eq!(
        frame.data.len(),
        frame.width as usize * frame.height as usize * 4
    );
    assert!(
        frame.data.chunks_exact(4).all(|p| p[3] == 255),
        "frame must be opaque"
    );
}
