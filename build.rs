//! Build script: embeds the Windows executable icon + version info.
//!
//! Only runs the resource compiler when building FOR a Windows target
//! (`CARGO_CFG_TARGET_OS`), so Linux/macOS/ARM cross-builds in CI pay zero
//! cost. `assets/icon.ico` is compiled into the PE's resource section, which
//! is what Explorer / the taskbar / shortcuts display — independent of the
//! runtime window icon set via `egui::ViewportBuilder::with_icon`.

fn main() {
    // Re-run only when the icon or this script changes.
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    // Version-info sheet shown in Explorer → Properties → Details.
    res.set("FileDescription", env!("CARGO_PKG_DESCRIPTION"));
    res.set("FileVersion", concat!(env!("CARGO_PKG_VERSION"), ".0"));
    res.set("ProductName", "Minecraft-MCP-RS");
    res.set("ProductVersion", concat!(env!("CARGO_PKG_VERSION"), ".0"));

    // Fail the build loudly if the icon is missing/corrupt on a Windows
    // target; non-Windows targets never reach here.
    if let Err(e) = res.compile() {
        panic!("failed to embed Windows resources (icon): {e}");
    }
}
