//! Parsing of what Windows' boot-time checker (autochk/chkdsk) wrote to the Application event log
//! (Wininit event 1001 / Chkdsk event 26212).

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// "Windows has scanned the file system and found no problems."
    NothingToFix,
    /// "Windows has made corrections to the file system."
    Fixed,
    /// errors were found that chkdsk could not repair
    ProblemsRemain,
    /// the event did not contain a recognisable outcome
    Unknown,
}

#[derive(Debug, Clone)]
pub struct BootCheck {
    pub letter: char,
    pub unix_time: i64,
    pub verdict: Verdict,
    pub details: Vec<String>,
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "2026-11-05T14:03:09.123456700Z" -> unix seconds.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

pub fn classify(text: &str) -> Verdict {
    let t = text.to_lowercase();
    let bad = [
        "could not fix", "cannot fix", "unable to fix", "cannot continue", "could not be repaired",
        "errors found", "found problems with the file system", "corrupt and unreadable", "failed",
    ];
    let any_bad = bad.iter().any(|b| t.contains(b));
    let fixed = t.contains("made corrections");
    let clean = t.contains("found no problems");
    if fixed {
        return Verdict::Fixed;
    }
    if clean {
        return Verdict::NothingToFix;
    }
    if any_bad {
        return Verdict::ProblemsRemain;
    }
    Verdict::Unknown
}

/// Lines worth showing the user about what was repaired / found.
pub fn details(text: &str) -> Vec<String> {
    let keys = [
        "recovered", "recovering", "corrected", "correcting", "repaired", "replaced", "deleting", "fixing", "orphan",
        "bad cluster", "bad sector", "unindexed", "errors found", "could not", "cannot", "made corrections", "no problems",
    ];
    let mut out: Vec<String> = vec![];
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        let low = l.to_lowercase();
        if keys.iter().any(|k| low.contains(k)) && !out.iter().any(|o| o == l) {
            out.push(l.to_string());
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// Drive letter from "Checking file system on E:" (case-insensitive); falls back to "repairing drive (E:".
pub fn drive_letter(text: &str) -> Option<char> {
    let low = text.to_lowercase();
    for marker in ["checking file system on ", "scanning and repairing drive (", "repairing drive ("] {
        if let Some(i) = low.find(marker) {
            let c = low[i + marker.len()..].chars().next()?;
            if c.is_ascii_alphabetic() {
                return Some(c.to_ascii_uppercase());
            }
        }
    }
    None
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)? + i;
    Some(&s[i..j])
}

fn unescape(s: &str) -> String {
    let s = s.replace("<![CDATA[", "").replace("]]>", "");
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#xD;", "")
        .replace("&#xA;", "\n")
        .replace("&amp;", "&")
}

/// Parse `wevtutil qe ... /f:xml` output (many <Event> elements) into per-drive boot checks.
pub fn parse_events(xml: &str) -> Vec<BootCheck> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(i) = rest.find("<Event") {
        let after = &rest[i..];
        let end = after.find("</Event>").map(|e| e + 8).unwrap_or(after.len());
        let ev = &after[..end];
        rest = &after[end..];
        let id = ev
            .find("<EventID")
            .and_then(|p| between(&ev[p..], ">", "</EventID>"))
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        if id != "1001" && id != "26212" {
            continue;
        }
        let time = ev
            .find("SystemTime=")
            .and_then(|p| ev.get(p + 12..p + 31))
            .and_then(parse_iso)
            .unwrap_or(0);
        let data = ev
            .find("<Data")
            .and_then(|p| between(&ev[p..], ">", "</Data>"))
            .map(unescape)
            .unwrap_or_default();
        let letter = drive_letter(&data).unwrap_or('?');
        out.push(BootCheck { letter, unix_time: time, verdict: classify(&data), details: details(&data) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = "Checking file system on E:\nThe type of the file system is exFAT.\nVolume label is Media.\n\nStage 1: Examining basic file system structure ...\nWindows has scanned the file system and found no problems.\nNo further action is required.";
    const FIXED: &str = "Checking file system on D:\nThe type of the file system is NTFS.\nCorrecting error in index $I30 for file 5.\nRecovering orphaned file X (12) into directory file 5.\nWindows has made corrections to the file system.\nNo further action is required.";
    const BAD: &str = "Checking file system on F:\nErrors found.  CHKDSK cannot continue in read-only mode.";

    #[test]
    fn iso_epoch() {
        assert_eq!(parse_iso("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso("2026-11-05T14:00:00Z"), Some(1793887200));
    }
    #[test]
    fn civil_leap() {
        assert_eq!(days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28), 2);
    }
    #[test]
    fn classify_clean() {
        assert_eq!(classify(CLEAN), Verdict::NothingToFix);
    }
    #[test]
    fn classify_fixed() {
        assert_eq!(classify(FIXED), Verdict::Fixed);
    }
    #[test]
    fn classify_bad() {
        assert_eq!(classify(BAD), Verdict::ProblemsRemain);
    }
    #[test]
    fn classify_unknown() {
        assert_eq!(classify("A disk check has been scheduled."), Verdict::Unknown);
    }
    #[test]
    fn letters() {
        assert_eq!(drive_letter(CLEAN), Some('E'));
        assert_eq!(drive_letter(FIXED), Some('D'));
        assert_eq!(drive_letter("Scanning and repairing drive (c): 5%"), Some('C'));
        assert_eq!(drive_letter("nothing"), None);
    }
    #[test]
    fn details_found() {
        let d = details(FIXED);
        assert!(d.iter().any(|l| l.contains("orphaned")));
        assert!(d.iter().any(|l| l.contains("made corrections")));
    }
    #[test]
    fn events_xml() {
        let xml = format!(
            "<Event xmlns='x'><System><EventID Qualifiers='0'>1001</EventID><TimeCreated SystemTime='2026-11-05T14:03:09.1Z'/></System><EventData><Data>{}</Data></EventData></Event>\n<Event><System><EventID>1000</EventID></System><EventData><Data>skip</Data></EventData></Event>\n<Event><System><EventID Qualifiers='0'>1001</EventID><TimeCreated SystemTime='2026-11-05T14:05:00.0Z'/></System><EventData><Data>{}</Data></EventData></Event>",
            CLEAN.replace('\n', "&#xA;"),
            FIXED.replace('\n', "&#xA;")
        );
        let v = parse_events(&xml);
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].letter, v[0].verdict.clone()), ('E', Verdict::NothingToFix));
        assert_eq!((v[1].letter, v[1].verdict.clone()), ('D', Verdict::Fixed));
        assert_eq!(v[0].unix_time, 1793887200 + 189);
    }
}
