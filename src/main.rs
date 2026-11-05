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
        "bootguard.exe <command>\n\
         \x20 list                     show every local drive (internal + external) and whether the boot check can repair it\n\
         \x20 arm [--drive E,F] [--dry-run]\n\
         \x20                          (run as Administrator) make the NEXT restart scan+repair all drives (or the listed ones)\n\
         \x20 status                   show what is armed\n\
         \x20 report [--no-wait] [--all]  after the reboot (--all = include older boots): per drive, was anything fixed or was there nothing to fix\n\
         \x20 disarm                   cancel: remove the report task and the armed state\n\
         Exit codes: 0 ok / nothing wrong, 1 problems remain or an action failed, 2 usage error, 3 not Administrator."
    );
}

fn parse_drive_list(v: &str) -> Result<Vec<char>, String> {
    let mut out = vec![];
    for p in v.split(',') {
        let c = p.trim().trim_end_matches(':').trim_end_matches('\\');
        let mut it = c.chars();
        match (it.next(), it.next()) {
            (Some(ch), None) if ch.is_ascii_alphabetic() => out.push(ch.to_ascii_uppercase()),
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
        if c1 != 0 && !is_dirty {
            println!("         {}", o1.trim().replace('\n', " "));
            code = 1;
        } else {
            armed.push(format!("{}:{}", d.letter, fs));
        }
    }
    println!("[4/4] one-shot report at your next logon");
    if dry {
        println!("      would create scheduled task {} -> \"{}\" report", TASK, exe.display());
    } else {
        let tr = format!("\"{}\" report", exe.display());
        let (c, o) = run("schtasks", &["/create", "/tn", TASK, "/tr", &tr, "/sc", "onlogon", "/rl", "highest", "/f"]);
        println!("      schtasks create -> exit {} {}", c, o.trim().replace('\n', " "));
        if c != 0 {
            code = 1;
        }
        let state = format!("armed_at={}\n{}\n", now_unix(), armed.iter().map(|a| format!("drive={}", a)).collect::<Vec<_>>().join("\n"));
        if std::fs::write(state_dir().join("armed.txt"), state).is_err() {
            println!("      WARNING: could not write the armed state file");
            code = 1;
        }
    }
    if armed.is_empty() {
        println!("\nNo drive could be armed.");
        return 1;
    }
    println!("\n{} {} drive(s): {}.", if dry { "DRY RUN - would arm" } else { "ARMED" }, armed.len(), armed.join("  "));
    if !dry {
        println!("Now RESTART Windows (Start > Power > Restart; a plain 'Shut down' uses Fast Startup and skips the check).");
        println!("On the boot screen Windows shows 'Scanning and repairing drive (X:): NN% complete' for every drive.");
        println!("At your next logon bootguard opens a window telling you, per drive, what was fixed or that nothing needed fixing.");
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
            println!("ARMED at unix {} ({} min ago) for: {}", at, (now_unix() - at) / 60, dr.join("  "));
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
    let (c, o) = run("schtasks", &["/delete", "/tn", TASK, "/f"]);
    println!("report task removed -> exit {} {}", c, o.trim().replace('\n', " "));
    if let Some((_, dr)) = read_armed() {
        let letters: Vec<String> = dr.iter().filter_map(|d| d.chars().next()).map(|c| format!("{}:", c)).collect();
        if !letters.is_empty() {
            let mut a: Vec<&str> = vec!["/x"];
            a.extend(letters.iter().map(|s| s.as_str()));
            let (c, o) = run("chkntfs", &a);
            println!("boot check cancelled for {} -> exit {} {}", letters.join(" "), c, o.trim().replace(char::from(10), " "));
            println!("(to make Windows check every drive at boot again later: chkntfs /d)");
        }
    }
    let _ = std::fs::remove_file(state_dir().join("armed.txt"));
    println!("armed state removed.");
    0
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
    let boot = if all { 120 } else { now_unix() - drives::uptime_secs() as i64 };
    let armed = read_armed();
    let (c, xml) = run("wevtutil", &["qe", "Application", "/q:*[System[Provider[@Name='Wininit']]]", "/c:200", "/rd:true", "/f:xml"]);
    // Online chkdsk scans also log here (event 26226); only used with --include-online (parser validation on real data).
    let (c2, xml2) = run("wevtutil", &["qe", "Application", "/q:*[System[Provider[@Name='Chkdsk']]]", "/c:200", "/rd:true", "/f:xml"]);
    if c != 0 && c2 != 0 {
        println!("Could not read the event log (exit {}): {}", c, xml.trim());
    }
    let mut events = parse::parse_events(&xml);
    // Chkdsk-provider events: online scans also log here, so outside preview mode only boot-scheduled checks count.
    events.extend(parse::parse_events(&xml2).into_iter().filter(|e| online || e.scheduled));
    let checks = latest_per_drive(events, boot - 120);

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
        text.push_str(&format!("UNCLEAR OUTCOME on: {}
", j(&unclear)));
    }
    if !missing.is_empty() {
        text.push_str(&format!("NOT CHECKED: {}\n", j(&missing)));
    }
    if letters.is_empty() {
        text.push_str("No boot-check records were found for this boot. Run 'bootguard arm' as Administrator, then Restart (not Shut down).\n");
    }
    print!("{}", text);
    if !all && !online {
        let _ = std::fs::write(state_dir().join("last-report.txt"), &text);
        println!("(saved to {})", state_dir().join("last-report.txt").display());
    } else {
        println!("(preview mode: nothing saved, task and armed state untouched)");
    }
    if armed.is_some() && !all && !online {
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(|s| s.as_str()) else {
        usage();
        std::process::exit(2)
    };
    let mut sel: Vec<char> = vec![];
    let (mut dry, mut wait, mut all, mut online) = (false, true, false, false);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => dry = true,
            "--no-wait" => wait = false,
            "--all" => all = true,
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
        "arm" => cmd_arm(sel, dry),
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
