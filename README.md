<div align="center">

# 🥾 bootguard

**Make your next reboot scan and repair every drive — internal and external, NTFS · exFAT · FAT · FAT32 — then tell you, drive by drive, whether anything was fixed.**

![Rust](https://img.shields.io/badge/Rust-std%20only-orange?logo=rust)
![Platform](https://img.shields.io/badge/Windows-10%2F11%20x64-blue?logo=windows)
![Filesystems](https://img.shields.io/badge/NTFS%20%7C%20exFAT%20%7C%20FAT%20%7C%20FAT32-supported-success)
![Tests](https://img.shields.io/badge/unit%20tests-12%20passing-brightgreen)

</div>

## How it works

1. **`bootguard arm`** (Administrator) finds every local fixed and removable drive, flags each repairable one for Windows' own
   boot-time checker (*autochk*), makes sure the check cannot be skipped (no countdown), and registers a one-shot logon task.
2. **You restart.** During boot Windows itself shows the live progress for every drive:
   `Scanning and repairing drive (E:): 37% complete` — then moves to the next drive.
3. **At logon** bootguard opens a report that reads the boot-check results back from the Windows event log and tells you, for
   **every drive**, one of: `FIXED`, `NOTHING TO FIX`, `PROBLEMS REMAIN`, or `NOT CHECKED` — plus what was repaired.

```text
=========== BOOTGUARD - boot-time disk check results ===========
Drive  Type                 Filesystem  Result
C:     fixed                NTFS        NOTHING TO FIX - scanned, no problems
E:     fixed                exFAT       FIXED - Windows found and repaired problems
         - Windows has made corrections to the file system.
Z:     removable/external   FAT32       NOTHING TO FIX - scanned, no problems
---------------------------------------------------------------
FIXED SOMETHING on: E:
NOTHING TO FIX on: C: Z:
```

> The percentages on the boot screen are Windows' own checker, which verifies drives one after another; bootguard does not
> replace the boot-time checker (that is what makes it safe) — it guarantees it runs on every drive and reports the outcome.

## Install

```powershell
irm https://github.com/Michaelunkai/bootguard-rs/releases/latest/download/install.ps1 | iex
```

or download `bootguard.exe` from [Releases](https://github.com/Michaelunkai/bootguard-rs/releases).

## Usage

```text
bootguard.exe list                       every local drive and whether the boot check can repair it
bootguard.exe arm [--drive E,F] [--dry-run]   (Administrator) make the NEXT restart scan + repair all (or listed) drives
bootguard.exe status                     what is armed
bootguard.exe report [--no-wait] [--all] per-drive result of the boot check (runs automatically at logon)
bootguard.exe disarm                     cancel: remove the logon task and exclude the armed drives from the boot check
```

Then **Restart** (Start → Power → *Restart*). A plain *Shut down* uses Fast Startup and skips the boot-time check.

Exit codes: `0` ok · `1` problems remain / action failed · `2` usage error · `3` not Administrator.

## Good to know

* External drives are included when connected at arm time **and** at boot; a drive that is absent is reported `NOT CHECKED`.
* Boot checks of large drives take a while — that is the price of a real, offline-quality repair.
* ReFS and unformatted/unready volumes cannot be checked at boot and are listed as skipped.
* `disarm` runs `chkntfs /x` for the armed drives; run `chkntfs /d` to restore Windows' default behaviour afterwards.
* Do not run other repair tools (for example `driveguard`) between `arm` and the restart — they clear the dirty flag you armed.

## Build

```bash
source .cargo-env.sh       # adjust toolchain paths for your machine
cargo test --release       # 12 unit tests (event-log parsing, verdicts, drive lists)
cargo build --release      # target/x86_64-pc-windows-gnu/release/bootguard.exe
```

No dependencies — only the Rust standard library and a few Win32 calls.

## License

MIT © Michael Fedorovsky
