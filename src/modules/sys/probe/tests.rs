use super::*;

/// Real output of `PROBE` on mbp-server (a laptop on AC power, macOS 27.0.1), 2026-10-09.
const SERVER: &str = include_str!("../fixtures/probe_mbp_server.txt");
/// A desktop Mac without a battery under thermal pressure, written from pmset's documented
/// output (`Thermal warning level set to N.`, `CPU_Speed_Limit = N`).
const DESKTOP: &str = include_str!("../fixtures/probe_desktop.txt");

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn the_fixtures_follow_the_script() {
    let markers: Vec<&str> = PROBE.lines().filter_map(|l| l.strip_prefix("echo '@@ ")?.strip_suffix('\'')).collect();
    assert_eq!(markers, ["host", "sysctl", "df", "batt", "therm", "now"]);
    for fixture in [SERVER, DESKTOP] {
        let found: Vec<&str> = fixture.lines().filter_map(|l| l.strip_prefix("@@ ")).collect();
        assert_eq!(found, markers);
    }
}

#[test]
fn reads_a_laptop_on_ac_power() {
    let s = parse(SERVER, 1_791_615_040);
    assert_eq!(s.host.as_deref(), Some("mbp-server"));
    assert!(close(s.cpu_load.unwrap(), [1.42, 2.06, 2.42]));
    assert_eq!(s.ncpu, Some(11));
    assert_eq!(s.mem_total, Some(19_327_352_832));
    assert_eq!(s.mem_free_pct, Some(73));
    assert_eq!(
        s.disks,
        [
            Disk {
                mount: "/".into(),
                device: "/dev/disk3s3s1".into(),
                total: 971_350_180 * 1024,
                used: 13_334_172 * 1024,
                avail: 856_126_932 * 1024,
                used_pct: Some(2),
            },
            Disk {
                mount: "/System/Volumes/Data".into(),
                device: "/dev/disk3s1".into(),
                total: 971_350_180 * 1024,
                used: 77_653_996 * 1024,
                avail: 856_126_932 * 1024,
                used_pct: Some(9),
            },
        ]
    );
    assert_eq!(
        s.battery,
        Some(Battery {
            percent: Some(100),
            state: Some("charged".into()),
            source: Some("AC Power".into()),
            remaining_mins: Some(0),
        })
    );
    let nominal = Thermal { nominal: true, ..Thermal::default() };
    assert_eq!(s.thermal, Some(nominal));
    // Uptime is on the probed machine's clock (`@@ now`), not the collector's.
    assert_eq!(s.uptime_secs, Some(1_791_615_036 - 1_791_307_644));
    assert_eq!(s.at, 1_791_615_040);
}

#[test]
fn reads_a_desktop_without_a_battery() {
    let s = parse(DESKTOP, 5);
    assert_eq!(s.host.as_deref(), Some("mac-pro"));
    assert_eq!(s.ncpu, Some(24));
    assert_eq!(s.disks.len(), 2);
    assert_eq!(s.disks[1].used_pct, Some(51));
    assert_eq!(s.battery, None);
    assert_eq!(
        s.thermal,
        Some(Thermal {
            nominal: false,
            warning_level: Some(1),
            performance_level: Some(0),
            cpu_speed_limit: Some(80),
        })
    );
    assert_eq!(s.uptime_secs, Some(90_000));
}

#[test]
fn reads_a_discharging_battery() {
    let text = "@@ batt\nNow drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t85%; discharging; 4:12 remaining present: true\n";
    let b = parse(text, 0).battery.unwrap();
    assert_eq!(
        b,
        Battery {
            percent: Some(85),
            state: Some("discharging".into()),
            source: Some("Battery Power".into()),
            remaining_mins: Some(252),
        }
    );
    let text = "@@ batt\n -InternalBattery-0 (id=1)\t7%; charging; (no estimate) present: true\n";
    let b = parse(text, 0).battery.unwrap();
    assert_eq!((b.percent, b.state.as_deref(), b.source, b.remaining_mins), (Some(7), Some("charging"), None, None));
    // A reshaped line keeps what it can.
    let b = parse("@@ batt\n -InternalBattery-0 full\n", 0).battery.unwrap();
    assert_eq!(b, Battery::default());
    let b = parse("@@ batt\n -InternalBattery-0\tlots; ; 9:xx left\n", 0).battery.unwrap();
    assert_eq!(b, Battery::default());
    let b = parse("@@ batt\n -InternalBattery-0\t50%; AC attached; x:10\n", 0).battery.unwrap();
    assert_eq!((b.percent, b.remaining_mins), (Some(50), None));
}

#[test]
fn missing_and_unknown_output_degrades_to_absent() {
    let empty = Snapshot { at: 7, ..Snapshot::default() };
    assert_eq!(parse("", 7), empty);
    assert_eq!(parse("garbage before any section\n@@ nothing\nx\n", 7), empty);
    // Every section present but empty: commands that failed.
    let blank = PROBE.lines().filter_map(|l| l.strip_prefix("echo '")?.strip_suffix('\'')).collect::<Vec<_>>().join("\n");
    assert_eq!(parse(&blank, 7), empty);
    let odd = "@@ host\n  \n@@ sysctl\nhw.ncpu: many\nhw.memsize: -1\nkern.memorystatus_level: 101\n\
               vm.loadavg: { 1.0 2.0 }\nkern.boottime: { usec = 4 }\nhw.ncpu.extra: 3\n\
               @@ df\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/x 1 2\n\
               /dev/y 10 5 5 n/a /Volumes/My Disk\n/dev/y 10 5 5 50% /Volumes/My Disk\n/dev/z 1 1 1 1%\n\
               @@ therm\nsomething new\n@@ now\nlater\n";
    let s = parse(odd, 7);
    assert_eq!((s.host, s.ncpu, s.mem_total, s.mem_free_pct, s.cpu_load, s.uptime_secs), (None, None, None, None, None, None));
    assert_eq!(s.thermal, None);
    // A mount with a space is one mount; a repeated mount and a line without one are skipped.
    let disk = Disk { mount: "/Volumes/My Disk".into(), device: "/dev/y".into(), total: 10_240, used: 5_120, avail: 5_120, used_pct: None };
    assert_eq!(s.disks, [disk]);
}

#[test]
fn uptime_needs_a_boot_before_now() {
    let text = "@@ sysctl\nkern.boottime: { sec = 100, usec = 0 }\n";
    assert_eq!(parse(text, 160).uptime_secs, Some(60));
    assert_eq!(parse(text, 50).uptime_secs, None);
    let loads = "@@ sysctl\nvm.loadavg: { 0.5 x 1 }\n";
    assert_eq!(parse(loads, 0).cpu_load, None);
}

#[test]
fn thermal_notes_and_levels() {
    let t = |text: &str| parse(&format!("@@ therm\n{text}"), 0).thermal;
    let nominal = Some(Thermal { nominal: true, ..Thermal::default() });
    assert_eq!(t("Note: No CPU power status has been recorded\n"), nominal);
    assert_eq!(t("Thermal warning level set to 0.\n"), Some(Thermal { nominal: true, warning_level: Some(0), ..Thermal::default() }));
    assert_eq!(t("Performance warning level set to 2.\n"), Some(Thermal { performance_level: Some(2), ..Thermal::default() }));
    assert_eq!(t("\tCPU_Speed_Limit \t= 100\n"), Some(Thermal { nominal: true, cpu_speed_limit: Some(100), ..Thermal::default() }));
}
