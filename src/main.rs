//! bootguard - make the NEXT reboot scan and repair every drive (internal + external, NTFS / exFAT / FAT / FAT32),
//! then, at logon, show for every drive whether anything was fixed or nothing needed fixing.
//!
//! The scan/repair itself is done by Windows' own boot-time checker (autochk), which shows its live
//! "Scanning and repairing drive (X:): NN% complete" progress on the boot screen. bootguard arms it for every
//! drive, makes sure it cannot be skipped, and reads back its results.
mod drives;
mod parse;

use drives::Drive;
use parse::{BootCheck, Verdict};
use std::process::Command;

const TASK: &str = "BootGuardReport";

fn run(cmd: &str, args: &[&str]) -> (i32, String) {
    match Command::new(cmd).args(args).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.code().unwrap_or(-1), s)
        }
        Err(e) => (-1, format!("cannot start {}: {}", cmd, e)),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn state_dir() -> std::path::PathBuf {
    let base = std::env::var_os("ProgramData").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("C:\\ProgramData"));
    let d = base.join("bootguard");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn usage() {
    println!(
        "bootguard.exe                no arguments = do everything: ask for Administrator, arm ALL drives for the next restart\n\
         bootguard.exe <command>\n\
         \x20 list                    show every local drive (internal + external) and whether the boot check can repair it\n\
         \x20 arm [--drive E,F] [--dry-run] [--pause]\n\
         \x20                          (run as Administrator) make the NEXT restart scan+repair all drives (or the listed ones)\n\
         \x20 status                   show what is armed\n\
         \x20 report [--no-wait] [--all] [--include-online]  after the reboot: per drive fixed / nothing to fix. A plain report also DISARMS (removes the logon task); --all / --include-online are read-only previews\n\
         \x20 disarm                   cancel: remove the report task and the armed state\n\
         Exit codes: 0 ok / nothing wrong, 1 problems remain or an action failed, 2 usage error, 3 not Administrator."
    );
}

/// A drive counts as armed only when chkntfs /c succeeded and the dirty flag is set (set succeeded or reads back dirty).
fn drive_armed(set_exit: i32, chkntfs_exit: i32, dirty_now: bool) -> bool {
    chkntfs_exit == 0 && (set_exit == 0 || dirty_now)
}

/// True when at least one boot-check record (already limited to events after arming) belongs to an armed drive;
/// only then has a restart happened and may the one-shot report disarm.
fn has_armed_record(armed: &[String], record_letters: &[char]) -> bool {
    record_letters.iter().any(|l| armed.iter().any(|a| a.chars().next() == Some(*l)))
}

/// Which flags each command accepts.
fn flag_allowed(cmd: &str, flag: &str) -> bool {
    match cmd {
        "arm" => matches!(flag, "--drive" | "--dry-run" | "--pause"),
        "report" => matches!(flag, "--no-wait" | "--all" | "--include-online"),
        _ => false,
    }
}

/// Start of the window in which boot-check events count: this boot, but never before the drives were armed.
fn report_since(preview: bool, boot: i64, armed_at: Option<i64>) -> i64 {
    if preview {
        return 0;
    }
    match armed_at {
        Some(a) => (boot - 120).max(a),
        None => boot - 120,
    }
}

/// Splits an armed-state entry such as "E:NTFS" into drive letter and filesystem.
fn split_armed(entry: &str) -> (char, String) {
    let mut it = entry.chars();
    let l = it.next().unwrap_or('?');
    (l, it.as_str().trim_start_matches(':').to_string())
}

/// One line per armed drive, e.g. "  E:  NTFS".
fn format_armed_lines(armed: &[String]) -> String {
    armed
        .iter()
        .map(|a| {
            let (l, fs) = split_armed(a);
            format!("  {}:  {}\n", l, if fs.is_empty() { "?" } else { &fs })
        })
        .collect()
}

/// The "what will happen on restart" block printed after a successful arm.
fn restart_summary(armed: &[String], report_task_ok: bool) -> String {
    let mut t = String::new();
    t.push_str("\n================ WHAT WILL HAPPEN ON RESTART ================\n");
    t.push_str("Windows will scan and repair these drives before it starts:\n");
    t.push_str(&format_armed_lines(armed));
    t.push_str("1. RESTART Windows now: Start > Power > Restart (a plain 'Shut down' uses Fast Startup and skips the check).\n");
    t.push_str("2. On the boot screen Windows shows 'Scanning and repairing drive (X:): NN% complete' for each drive above;\n");
    t.push_str("   when every drive is done it returns to Windows by itself - you do not need to press anything.\n");
    if report_task_ok {
        t.push_str("3. At logon a bootguard report window appears: per drive FIXED / NOTHING TO FIX / PROBLEMS REMAIN / NOT CHECKED.\n");
    } else {
        t.push_str("3. NO report window will appear at logon (the logon task/state could not be set up) - run 'bootguard report' yourself after the restart.\n");
    }
    t.push_str("==============================================================\n");
    t
}

fn parse_drive_list(v: &str) -> Result<Vec<char>, String> {
    let mut out = vec![];
    for p in v.split(',') {
        let c = p.trim().trim_end_matches(':').trim_end_matches('\\');
        let mut it = c.chars();
        match (it.next(), it.next()) {
            (Some(ch), None) if ch.is_ascii_alphabetic() => {
                let u = ch.to_ascii_uppercase();
                if !out.contains(&u) {
                    out.push(u);
                }
            }
            _ => return Err(format!("bad drive letter '{}'", p)),
        }
    }
    Ok(out)
}

fn targets(sel: &[char]) -> Vec<Drive> {
    let all = drives::list();
    if sel.is_empty() {
        all
    } else {
        sel.iter().filter_map(|l| all.iter().find(|d| d.letter == *l).cloned()).collect()
    }
}

fn cmd_list() -> i32 {
    println!("Drive  Type                 Filesystem  Boot check");
    for d in drives::list() {
        println!(
            "{}:     {:<20} {:<11} {}",
            d.letter,
            d.kind_name(),
            d.fs.clone().unwrap_or_else(|| "(not ready)".into()),
            if d.checkable() { "can scan + repair at boot" } else { "cannot be checked at boot (unsupported or not ready)" }
        );
    }
    0
}

fn bootexecute_ok() -> (bool, String) {
    let (_, out) = run("reg", &["query", "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager", "/v", "BootExecute"]);
    let low = out.to_lowercase();
    (low.contains("autocheck autochk"), out.trim().replace('\n', " "))
}

fn cmd_arm(sel: Vec<char>, dry: bool) -> i32 {
    let exe = std::env::current_exe().unwrap_or_default();
    if !dry && !drives::is_admin() {
        eprintln!("bootguard arm must run as Administrator (right-click PowerShell > Run as administrator). Nothing was changed.");
        return 3;
    }
    let ds = targets(&sel);
    if ds.is_empty() {
        eprintln!("no matching local drives found");
        return 1;
    }
    let mut code = 0;
    let (ok, raw) = bootexecute_ok();
    println!("[1/4] boot-time checker: {}", if ok { "autochk is registered in BootExecute".to_string() } else { format!("WARNING: autochk not found in BootExecute ({}). The reboot check will NOT run until it is restored.", raw) });
    if !ok {
        code = 1;
    }
    println!("[2/4] making the boot check mandatory (restore default, no skip countdown)");
    for args in [&["/d"][..], &["/t:0"][..]] {
        if dry {
            println!("      would run: chkntfs {}", args.join(" "));
        } else {
            let (c, o) = run("chkntfs", args);
            println!("      chkntfs {} -> exit {} {}", args.join(" "), c, o.trim().replace('\n', " "));
        }
    }
    println!("[3/4] arming drives for the next restart");
    let mut armed = vec![];
    let mut not_armed: Vec<char> = vec![];
    for d in &ds {
        let fs = d.fs.clone().unwrap_or_default();
        if !d.checkable() {
            println!("      {}: SKIPPED ({} - {})", d.letter, d.kind_name(), if fs.is_empty() { "not ready".to_string() } else { format!("{} is not repairable at boot", fs) });
            continue;
        }
        if dry {
            println!("      {}: would run: fsutil dirty set {}:  and  chkntfs /c {}:   ({}, {})", d.letter, d.letter, d.letter, fs, d.kind_name());
            armed.push(format!("{}:{}", d.letter, fs));
            continue;
        }
        let (c1, o1) = run("fsutil", &["dirty", "set", &format!("{}:", d.letter)]);
        let (c2, _) = run("chkntfs", &["/c", &format!("{}:", d.letter)]);
        let (_, q) = run("fsutil", &["dirty", "query", &format!("{}:", d.letter)]);
        let is_dirty = !q.to_lowercase().contains("not dirty") && q.to_lowercase().contains("dirty");
        println!("      {}: {} {} -> dirty-set exit {} (now {}), chkntfs /c exit {}", d.letter, fs, d.kind_name(), c1, if is_dirty { "flagged for check" } else { "NOT flagged" }, c2);
        if drive_armed(c1, c2, is_dirty) {
            armed.push(format!("{}:{}", d.letter, fs));
        } else {
            println!("      !!! {}: NOT ARMED - will NOT be checked at the next restart. {}", d.letter, o1.trim().replace('\n', " "));
            not_armed.push(d.letter);
            code = 1;
        }
    }
    if armed.is_empty() {
        println!("\nERROR: No drive could be armed. Nothing will be checked at the next restart. (No logon task was created.)");
        return 1;
    }
    println!("[4/4] one-shot report at your next logon");
    let mut task_ok = true;
    if dry {
        println!("      would create scheduled task {} -> \"{}\" report", TASK, exe.display());
    } else {
        let tr = format!("\"{}\" report", exe.display());
        let (c, o) = run("schtasks", &["/create", "/tn", TASK, "/tr", &tr, "/sc", "onlogon", "/rl", "highest", "/f"]);
        println!("      schtasks create -> exit {} {}", c, o.trim().replace('\n', " "));
        if c != 0 {
            println!("      !!! ERROR: the logon report task could NOT be created (schtasks exit {}). Drives may be flagged, but NO report window will appear at logon.", c);
            code = 1;
            task_ok = false;
        }
        let state = format!("armed_at={}\n{}\n", now_unix(), armed.iter().map(|a| format!("drive={}", a)).collect::<Vec<_>>().join("\n"));
        if std::fs::write(state_dir().join("armed.txt"), state).is_err() {
            println!("      !!! ERROR: could not write the armed state file in {}", state_dir().display());
            code = 1;
            task_ok = false;
        }
    }
    println!("\n{} {} drive(s): {}.", if dry { "DRY RUN - would arm" } else { "ARMED" }, armed.len(), armed.join("  "));
    if !not_armed.is_empty() {
        println!("NOT armed (will not be checked): {}", not_armed.iter().map(|c| format!("{}:", c)).collect::<Vec<_>>().join(" "));
    }
    if code != 0 && !dry {
        println!("\n!!! WARNING: ARMING IS INCOMPLETE - see the errors above. Fix the problem and run bootguard again, or run 'bootguard disarm'.");
    }
    if !dry {
        print!("{}", restart_summary(&armed, task_ok));
    }
    code
}

fn read_armed() -> Option<(i64, Vec<String>)> {
    let t = std::fs::read_to_string(state_dir().join("armed.txt")).ok()?;
    let mut at = 0;
    let mut dr = vec![];
    for l in t.lines() {
        if let Some(v) = l.strip_prefix("armed_at=") {
            at = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = l.strip_prefix("drive=") {
            dr.push(v.to_string());
        }
    }
    Some((at, dr))
}

fn cmd_status() -> i32 {
    match read_armed() {
        Some((at, dr)) => {
            println!("ARMED at unix {} ({} min ago) for {} drive(s):", at, (now_unix() - at) / 60, dr.len());
            print!("{}", format_armed_lines(&dr));
            let (c, _) = run("schtasks", &["/query", "/tn", TASK]);
            println!("report task {}: {}", TASK, if c == 0 { "present" } else { "missing" });
        }
        None => println!("not armed"),
    }
    let (ok, _) = bootexecute_ok();
    println!("autochk in BootExecute: {}", if ok { "yes" } else { "NO" });
    0
}

fn cmd_disarm() -> i32 {
    if !drives::is_admin() {
        eprintln!("bootguard disarm must run as Administrator. Nothing was changed.");
        return 3;
    }
    let mut code = 0;
    let (c, o) = run("schtasks", &["/delete", "/tn", TASK, "/f"]);
    let still = run("schtasks", &["/query", "/tn", TASK]).0 == 0;
    println!("report task removed -> exit {} {}", c, o.trim().replace('\n', " "));
    if still {
        println!("!!! ERROR: the report task {} is still present.", TASK);
        code = 1;
    }
    if let Some((_, dr)) = read_armed() {
        let letters: Vec<String> = dr.iter().filter_map(|d| d.chars().next()).map(|c| format!("{}:", c)).collect();
        if !letters.is_empty() {
            let mut a: Vec<&str> = vec!["/x"];
            a.extend(letters.iter().map(|s| s.as_str()));
            let (c, o) = run("chkntfs", &a);
            println!("boot check cancelled for {} -> exit {} {}", letters.join(" "), c, o.trim().replace('\n', " "));
            if c != 0 {
                println!("!!! ERROR: chkntfs /x failed.");
                code = 1;
            } else {
                println!("NOTE: {} are now EXCLUDED from Windows' automatic boot checks until you run 'chkntfs /d' or 'bootguard arm'.", letters.join(" "));
            }
        }
    }
    if code == 0 {
        let _ = std::fs::remove_file(state_dir().join("armed.txt"));
        println!("armed state removed.");
    } else {
        println!("armed state KEPT because disarm was incomplete; fix the errors above and run disarm again.");
    }
    code
}

/// Latest boot-check event per drive that happened during this boot.
fn latest_per_drive(mut v: Vec<BootCheck>, since: i64) -> Vec<BootCheck> {
    v.retain(|c| c.unix_time >= since);
    v.sort_by_key(|c| c.unix_time);
    let mut out: Vec<BootCheck> = vec![];
    for c in v {
        if let Some(p) = out.iter_mut().find(|o| o.letter == c.letter && c.letter != '?') {
            *p = c;
        } else {
            out.push(c);
        }
    }
    out
}

fn cmd_report(wait: bool, all: bool, online: bool) -> i32 {
    let armed = read_armed();
    let preview = all || online;
    let boot = now_unix() - drives::uptime_secs() as i64;
    let since = report_since(preview, boot, armed.as_ref().map(|a| a.0));
    let (c, xml) = run("wevtutil", &["qe", "Application", "/q:*[System[Provider[@Name='Wininit']]]", "/c:200", "/rd:true", "/f:xml"]);
    // Online chkdsk scans also log here (event 26226); only used with --include-online (parser validation on real data).
    let (c2, xml2) = run("wevtutil", &["qe", "Application", "/q:*[System[Provider[@Name='Chkdsk']]]", "/c:200", "/rd:true", "/f:xml"]);
    if c != 0 && c2 != 0 {
        println!("Could not read the event log (exit {}): {}", c, xml.trim());
    }
    let mut events = parse::parse_events(&xml);
    // Chkdsk-provider events: online scans also log here, so outside preview mode only boot-scheduled checks count.
    events.extend(parse::parse_events(&xml2).into_iter().filter(|e| online || e.scheduled));
    let checks = latest_per_drive(events, since);
    if let (Some((_, dr)), false) = (&armed, preview) {
        let has = has_armed_record(dr, &checks.iter().map(|c| c.letter).collect::<Vec<_>>());
        if !has {
            println!("No boot-check results since the drives were armed - Windows has not restarted (and run the check) yet.");
            println!("Still armed: the logon task and armed state are kept. RESTART Windows (not Shut down), then this report appears again.");
            pause_if_asked(wait);
            return 1;
        }
    }

    let present = drives::list();
    let wanted: Vec<char> = match &armed {
        Some((_, dr)) => dr.iter().filter_map(|d| d.chars().next()).collect(),
        None => checks.iter().map(|c| c.letter).filter(|c| *c != '?').collect(),
    };
    let mut fixed = vec![];
    let mut clean = vec![];
    let mut problems = vec![];
    let mut missing = vec![];
    let mut unclear = vec![];
    let mut text = String::new();
    text.push_str("=========== BOOTGUARD - boot-time disk check results ===========\n");
    text.push_str("Drive  Type                 Filesystem  Result\n");
    let mut letters: Vec<char> = wanted.clone();
    for c in &checks {
        if c.letter != '?' && !letters.contains(&c.letter) {
            letters.push(c.letter);
        }
    }
    letters.sort();
    letters.dedup();
    for l in &letters {
        let d = present.iter().find(|d| d.letter == *l);
        let (kind, fs) = d.map(|d| (d.kind_name().to_string(), d.fs.clone().unwrap_or_else(|| "?".into()))).unwrap_or(("(not present now)".into(), "?".into()));
        let (_, q) = run("fsutil", &["dirty", "query", &format!("{}:", l)]);
        let dirty_now = q.to_lowercase().contains("dirty") && !q.to_lowercase().contains("not dirty");
        let res = match checks.iter().find(|c| c.letter == *l) {
            Some(c) => match c.verdict {
                Verdict::Fixed => {
                    fixed.push(*l);
                    "FIXED - Windows found and repaired problems".to_string()
                }
                Verdict::NothingToFix => {
                    clean.push(*l);
                    "NOTHING TO FIX - scanned, no problems".to_string()
                }
                Verdict::ProblemsRemain => {
                    problems.push(*l);
                    "PROBLEMS REMAIN - could not be fully repaired".to_string()
                }
                Verdict::Unknown => {
                    unclear.push(*l);
                    "CHECKED, but the log text has no clear outcome (see details)".to_string()
                }
            },
            None => {
                missing.push(*l);
                "NOT CHECKED - no boot-check record (drive absent at boot, or restart was skipped/Fast Startup)".to_string()
            }
        };
        text.push_str(&format!("{}:     {:<20} {:<11} {}{}\n", l, kind, fs, res, if dirty_now { "  [still flagged dirty now]" } else { "" }));
        if let Some(c) = checks.iter().find(|c| c.letter == *l) {
            for dl in &c.details {
                text.push_str(&format!("         - {}\n", dl));
            }
        }
    }
    text.push_str("---------------------------------------------------------------\n");
    let j = |v: &Vec<char>| v.iter().map(|c| format!("{}:", c)).collect::<Vec<_>>().join(" ");
    if !fixed.is_empty() {
        text.push_str(&format!("FIXED SOMETHING on: {}\n", j(&fixed)));
    }
    if !clean.is_empty() {
        text.push_str(&format!("NOTHING TO FIX on: {}\n", j(&clean)));
    }
    if !problems.is_empty() {
        text.push_str(&format!("PROBLEMS REMAIN on: {}\n", j(&problems)));
    }
    if !unclear.is_empty() {
        text.push_str(&format!("UNCLEAR OUTCOME on: {}\n", j(&unclear)));
    }
    if !missing.is_empty() {
        text.push_str(&format!("NOT CHECKED: {}\n", j(&missing)));
    }
    if letters.is_empty() {
        text.push_str("No boot-check records were found for this boot. Run 'bootguard arm' as Administrator, then Restart (not Shut down).\n");
    }
    print!("{}", text);
    if !preview {
        let _ = std::fs::write(state_dir().join("last-report.txt"), &text);
        println!("(saved to {})", state_dir().join("last-report.txt").display());
    } else {
        println!("(preview mode: nothing saved, task and armed state untouched)");
    }
    if armed.is_some() && !preview {
        let _ = run("schtasks", &["/delete", "/tn", TASK, "/f"]);
        let _ = std::fs::remove_file(state_dir().join("armed.txt"));
        println!("One-shot report finished; the logon task was removed. Run 'bootguard arm' again before another reboot check.");
    }
    if wait {
        println!("\nPress Enter to close.");
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    }
    if problems.is_empty() && missing.is_empty() && unclear.is_empty() && !letters.is_empty() {
        0
    } else {
        1
    }
}

fn pause_if_asked(ask: bool) {
    if ask {
        println!("\nPress Enter to close.");
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    }
}

/// True for the panic message println! raises when stdout is a closed pipe (e.g. `bootguard status | head -1`).
fn is_stdout_closed_panic(msg: &str) -> bool {
    msg.contains("failed printing to stdout")
}

/// Exit quietly (no panic text) when the reader of our stdout went away; other panics keep the default report.
fn install_quiet_pipe_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let p = info.payload();
        let msg = p.downcast_ref::<String>().map(|s| s.as_str()).or_else(|| p.downcast_ref::<&str>().copied()).unwrap_or("");
        if is_stdout_closed_panic(msg) {
            std::process::exit(0);
        }
        default(info);
    }));
}

fn main() {
    install_quiet_pipe_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Bare `bootguard.exe` = do everything: arm all drives for the next restart (asks for Administrator itself).
    if args.is_empty() {
        if !drives::is_admin() {
            println!("bootguard needs Administrator rights to arm the boot-time check - accept the Windows (UAC) prompt...");
            if drives::relaunch_elevated("arm --pause") {
                println!("An elevated bootguard window was opened; it arms every drive and shows the result.");
                std::process::exit(0);
            }
            eprintln!("The Administrator request was declined or failed. Nothing was changed.");
            std::process::exit(3);
        }
        let code = cmd_arm(vec![], false);
        pause_if_asked(true);
        std::process::exit(code);
    }
    let Some(cmd) = args.first().map(|s| s.as_str()) else {
        usage();
        std::process::exit(2)
    };
    let mut sel: Vec<char> = vec![];
    let (mut dry, mut wait, mut all, mut online, mut pause) = (false, true, false, false, false);
    let mut i = 1;
    while i < args.len() {
        if args[i].starts_with("--") && !flag_allowed(cmd, args[i].as_str()) {
            eprintln!("'{}' is not valid for '{}'", args[i], cmd);
            usage();
            std::process::exit(2);
        }
        match args[i].as_str() {
            "--dry-run" => dry = true,
            "--no-wait" => wait = false,
            "--all" => all = true,
            "--pause" => pause = true,
            "--include-online" => online = true,
            "--drive" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("--drive needs a value");
                    std::process::exit(2)
                };
                match parse_drive_list(v) {
                    Ok(l) => sel = l,
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(2)
                    }
                }
            }
            o => {
                eprintln!("unknown argument: {}", o);
                usage();
                std::process::exit(2)
            }
        }
        i += 1;
    }
    let code = match cmd {
        "list" => cmd_list(),
        "arm" => {
            let c = cmd_arm(sel, dry);
            pause_if_asked(pause);
            c
        }
        "status" => cmd_status(),
        "report" => cmd_report(wait, all, online),
        "disarm" => cmd_disarm(),
        "-h" | "--help" | "help" => {
            usage();
            0
        }
        o => {
            eprintln!("unknown command: {}", o);
            usage();
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_list_ok() {
        assert_eq!(parse_drive_list("e, F:,z\\").unwrap(), vec!['E', 'F', 'Z']);
    }
    #[test]
    fn split_armed_entry() {
        assert_eq!(split_armed("E:NTFS"), ('E', "NTFS".to_string()));
        assert_eq!(split_armed("F:exFAT"), ('F', "exFAT".to_string()));
        assert_eq!(split_armed("G"), ('G', String::new()));
    }
    #[test]
    fn armed_lines_format() {
        let l = format_armed_lines(&["C:NTFS".to_string(), "E:FAT32".to_string()]);
        assert_eq!(l, "  C:  NTFS\n  E:  FAT32\n");
    }
    #[test]
    fn restart_summary_lists_drives_and_steps() {
        let t = restart_summary(&["C:NTFS".to_string(), "E:exFAT".to_string()], true);
        assert!(t.contains("C:  NTFS") && t.contains("E:  exFAT"));
        assert!(t.contains("RESTART") && t.contains("NN% complete") && t.contains("by itself") && t.contains("report window"));
    }
    #[test]
    fn closed_pipe_panic_detected() {
        assert!(is_stdout_closed_panic("failed printing to stdout: The pipe is being closed. (os error 232)"));
        assert!(!is_stdout_closed_panic("index out of bounds"));
    }
    #[test]
    fn armed_decision() {
        assert!(drive_armed(0, 0, false));
        assert!(drive_armed(1, 0, true));
        assert!(!drive_armed(0, 1, true));
        assert!(!drive_armed(1, 0, false));
    }
    #[test]
    fn flags_per_command() {
        assert!(flag_allowed("arm", "--dry-run") && flag_allowed("report", "--all"));
        assert!(!flag_allowed("disarm", "--dry-run") && !flag_allowed("list", "--drive") && !flag_allowed("arm", "--all"));
    }
    #[test]
    fn report_window_never_before_arming() {
        assert_eq!(report_since(false, 1000, Some(5000)), 5000);
        assert_eq!(report_since(false, 9000, Some(5000)), 8880);
        assert_eq!(report_since(false, 9000, None), 8880);
        assert_eq!(report_since(true, 9000, Some(5000)), 0);
    }
    #[test]
    fn summary_without_task_does_not_promise_report() {
        let t = restart_summary(&["C:NTFS".to_string()], false);
        assert!(t.contains("NO report window") && !t.contains("report window appears"));
    }
    #[test]
    fn armed_record_detection() {
        let a = vec!["C:NTFS".to_string(), "E:exFAT".to_string()];
        assert!(has_armed_record(&a, &['E']));
        assert!(!has_armed_record(&a, &['F', '?']));
        assert!(!has_armed_record(&a, &[]));
    }
    #[test]
    fn drive_list_dedupes() {
        assert_eq!(parse_drive_list("E,e:,E").unwrap(), vec!['E']);
    }
    #[test]
    fn drive_list_bad() {
        assert!(parse_drive_list("EE").is_err());
        assert!(parse_drive_list("1").is_err());
    }
    #[test]
    fn latest_wins_and_old_dropped() {
        let mk = |l, t, v| BootCheck { letter: l, unix_time: t, verdict: v, details: vec![], provider: String::new(), scheduled: false };
        let v = vec![mk('E', 10, Verdict::ProblemsRemain), mk('E', 50, Verdict::Fixed), mk('D', 5, Verdict::NothingToFix), mk('C', 100, Verdict::NothingToFix)];
        let r = latest_per_drive(v, 20);
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|c| c.letter == 'E' && c.verdict == Verdict::Fixed));
        assert!(r.iter().any(|c| c.letter == 'C'));
    }
}
