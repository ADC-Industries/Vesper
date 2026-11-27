//! The server's integration tests, built as ONE test binary.
//!
//! Every file here used to be its own `tests/*.rs` binary, and each one linked the whole server
//! (GPU rendering, FFmpeg, IDN) with full debug info, so building the tests was mostly linking.
//! One binary links once. GPU sharing is already process-wide (see `crates/render`), and each
//! test starts its own server on its own ports, so the modules can run side by side.
//!
//! Run one file's tests with a module filter, for example
//! `sg render -c "cargo test -p vesper-server --test server site::"`.

mod common;

/// `IDN_VULKAN_ENCODE=off` for the whole test binary, as `vesper`'s `main` sets it (see
/// [`vesper_server::vulkan_encode_off`]; M-4). Set from an ELF constructor: it runs before
/// `main`, while the process has a single thread, so setting the environment is sound. (On
/// other platforms IDN's probe makes no Vulkan device.)
#[cfg(target_os = "linux")]
#[used]
#[unsafe(link_section = ".init_array")]
static VULKAN_ENCODE_OFF: extern "C" fn() = {
    extern "C" fn off() {
        vesper_server::vulkan_encode_off();
    }
    off
};

mod api;
mod audio;
mod builds;
mod captions;
mod chords;
mod codex;
mod displays;
mod editing;
mod idn_e2e;
mod images;
mod language;
mod lifecycle;
mod live_data;
mod live_sound;
mod live_sources;
mod looks;
mod masks;
mod mdns;
mod media_bins;
mod media_live;
mod omt;
mod pairing;
mod presence;
mod remote_compat;
mod reports;
mod screen_adjust;
mod screen_config;
mod screen_layouts;
mod scripture;
mod show_gaps;
mod show_options;
mod signage;
mod site;
mod soak;
mod stage;
mod streaming;
mod themes;
mod translate;
mod updates;
mod video;
mod websocket;
