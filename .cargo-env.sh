# Build environment for this host (source this, then `cargo build --release`).
# Toolchain lives on F: ; MinGW gcc is the linker (no MSVC installed).
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUSTUP_HOME=F:/backup/rust/rustup CARGO_HOME=F:/backup/rust/cargo PATH="/f/backup/rust/cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HERE/target" CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=F:/backup/windowsapps/installed/scoop/apps/mingw/current/bin/gcc.exe PATH="/f/backup/windowsapps/installed/scoop/apps/mingw/current/bin:$PATH"
export CARGO_BUILD_TARGET=x86_64-pc-windows-gnu
