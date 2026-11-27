//! GPU errors and device loss (a separate test binary: losing the device on purpose must not
//! disturb the other render tests, which share the process's device).

use model::{Color, Layer};
use render::{LayerContent, Renderer, RendererOptions, ScreenConfig};
use std::sync::{Mutex, MutexGuard};

/// The tests here change the process's shared device: one at a time.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn fill(c: Color) -> [LayerContent<'static>; 1] {
    [LayerContent::color(Layer::Slide, c)]
}

const RED: Color = Color::rgb(1.0, 0.0, 0.0);
const GREEN: Color = Color::rgb(0.0, 1.0, 0.0);
const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);

/// A submit that reports a GPU error queues nothing: the next frame waited for is the next
/// one submitted, not the failed one (A02-1).
#[test]
fn a_submit_that_fails_queues_no_frame_for_readback() {
    let _g = serial();
    let mut r = Renderer::new(RendererOptions::default()).expect("renderer");
    let mut screen = r.create_screen(ScreenConfig::opaque(32, 18));
    r.submit(&mut screen, &fill(RED)).unwrap();
    assert_eq!(
        r.wait(&mut screen).unwrap().unwrap().pixel(4, 4),
        [255, 0, 0, 255]
    );

    r.raise_gpu_error_for_tests("out of memory (simulated)");
    let e = r.submit(&mut screen, &fill(GREEN)).unwrap_err();
    assert!(format!("{e:#}").contains("simulated"), "{e:#}");
    assert_eq!(screen.pending(), 0, "the failed frame is not in flight");
    assert!(r.wait(&mut screen).unwrap().is_none());

    r.submit(&mut screen, &fill(BLUE)).unwrap();
    let f = r.wait(&mut screen).unwrap().unwrap();
    assert_eq!(f.pixel(4, 4), [0, 0, 255, 255], "the frame just submitted");
}

/// After the device is lost, the renderer moves to a new device and its screens render
/// again (A02-2).
#[test]
fn a_lost_device_is_replaced_and_screens_render_again() {
    let _g = serial();
    let mut r = Renderer::new(RendererOptions::default()).expect("renderer");
    let mut screen = r.create_screen(ScreenConfig::opaque(32, 18));
    r.submit(&mut screen, &fill(RED)).unwrap();
    r.wait(&mut screen).unwrap().unwrap();
    let before = r.render_layers((16, 16), &fill(RED)).unwrap();
    assert_eq!(before.pixel(2, 2), [255, 0, 0, 255]);

    r.lose_device_for_tests();
    assert!(r.is_lost());
    // The engine's periodic check: the device is replaced and every cache dropped.
    assert!(r.recover().unwrap());
    assert!(!r.is_lost());
    assert!(!r.recover().unwrap(), "nothing more to do");

    // The screen made on the old device is made again on the new one by its next submit.
    r.submit(&mut screen, &fill(GREEN)).unwrap();
    let f = r.wait(&mut screen).unwrap().unwrap();
    assert_eq!(f.pixel(4, 4), [0, 255, 0, 255]);
    let after = r.render_layers((16, 16), &fill(BLUE)).unwrap();
    assert_eq!(after.pixel(2, 2), [0, 0, 255, 255]);

    // A renderer that never calls `recover` heals in `submit`.
    let mut other = Renderer::new(RendererOptions::default()).expect("renderer");
    let mut s2 = other.create_screen(ScreenConfig::opaque(8, 8));
    other.lose_device_for_tests();
    other.submit(&mut s2, &fill(RED)).unwrap();
    assert_eq!(
        other.wait(&mut s2).unwrap().unwrap().pixel(1, 1),
        [255, 0, 0, 255]
    );
    // ... and so do renderers made after the loss.
    let mut late = Renderer::new(RendererOptions::default()).expect("renderer");
    assert!(!late.is_lost());
    assert_eq!(
        late.render_layers((8, 8), &fill(GREEN))
            .unwrap()
            .pixel(1, 1),
        [0, 255, 0, 255]
    );
}

/// D03-7: a frame still in flight on a device the renderer has since replaced (another
/// screen's submit noticed the loss first) is dropped by `wait`, not waited for forever.
#[test]
fn waiting_on_a_frame_from_a_replaced_device_does_not_hang() {
    let _g = serial();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut r = Renderer::new(RendererOptions::default()).expect("renderer");
        let mut one = r.create_screen(ScreenConfig::opaque(16, 16));
        let mut two = r.create_screen(ScreenConfig::opaque(16, 16));
        r.submit(&mut one, &fill(RED)).unwrap();
        r.lose_device_for_tests();
        // Screen two's submit replaces the device; screen one's frame was mapped on the old.
        r.submit(&mut two, &fill(GREEN)).unwrap();
        let dropped = r.wait(&mut one).unwrap();
        let two_frame = r.wait(&mut two).unwrap().map(|f| f.pixel(1, 1));
        // Screen one renders again on the new device.
        r.submit(&mut one, &fill(BLUE)).unwrap();
        let again = r.wait(&mut one).unwrap().map(|f| f.pixel(1, 1));
        let _ = tx.send((dropped.is_none(), two_frame, again));
    });
    let (dropped, two, again) = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("wait hung on a frame of the replaced device");
    assert!(dropped);
    assert_eq!(two, Some([0, 255, 0, 255]));
    assert_eq!(again, Some([0, 0, 255, 255]));
}

/// E03-1: a device replaced by a screen's submit (not by an explicit `recover`) moves the
/// device generation, so a caller that renders only on change sees that the frames other
/// screens had in flight were dropped and draws those screens again.
#[test]
fn a_device_replaced_by_a_submit_moves_the_generation() {
    let _g = serial();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut r = Renderer::new(RendererOptions::default()).expect("renderer");
        let mut one = r.create_screen(ScreenConfig::opaque(16, 16));
        let mut two = r.create_screen(ScreenConfig::opaque(16, 16));
        let start = r.device_generation();
        r.submit(&mut one, &fill(RED)).unwrap();
        r.lose_device_for_tests();
        r.submit(&mut two, &fill(GREEN)).unwrap();
        let dropped = r.wait(&mut one).unwrap().is_none();
        let moved = r.device_generation() != start;
        // The explicit check afterwards finds nothing to do: only the generation tells.
        let recovered_again = r.recover().unwrap();
        let _ = tx.send((dropped, moved, recovered_again));
    });
    let (dropped, moved, recovered_again) = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("test hung");
    assert!(dropped, "screen one's frame was lost with the device");
    assert!(moved, "the generation moved with the replacement");
    assert!(!recovered_again);
}
