//! Volume-level probes for hidden-space accounting (C29).
//!
//! Three questions, three unprivileged probes, each shaped like `snapshots.rs`:
//!
//! 1. **How much of this volume is used?** [`volume_used`] is statfs arithmetic,
//!    `(f_blocks − f_bavail) × f_bsize` — the figure `df` and `diskutil` corroborate.
//!    On APFS those are container-wide numbers, which is exactly what a scan of `/`
//!    should be compared against. Not sysinfo's `available_space`, which on macOS
//!    includes the purgeable pool and would understate used by that whole figure.
//! 2. **What do the *other* volumes of this container hold?** The decomposition this
//!    question feeds is **held** (2026-11-03): its acceptance run was underpowered and
//!    the external-volume tie-breaker has not run, so nothing in the shipped panel asks
//!    it. The machinery stays — [`parse_containers`], [`decompose_other_volumes`],
//!    [`container_report`] and their tests — because it is exactly what that cross-check
//!    will run, and the acceptance harness (`examples/hidden_measure.rs`) still drives
//!    it. It runs `diskutil apfs list -plist` with a **fixed argv, no interpolation**
//!    (the C27 `tmutil` rule), parses the plist as data, and refuses rather than
//!    half-parses.
//! 3. **How much can macOS free on its own?** [`purgeable_bytes`] is the difference
//!    between two capacity figures the OS reports (`NSURL` resource values). It is an
//!    annotation — "up to N" — never a slice of the hidden residual, because much of
//!    that pool is space the tree already counts.
//!
//! Snapshots are counted, never sized (C27), by the snapshots panel of C27; the
//! hidden-space panel no longer carries a count while the decomposition is held.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// The Data volume's own mount, which the merged namespace already presents through
/// firmlinks. Apple's fixed path, not disk-derived. The device bound cannot stop it
/// (it shares the boot group's `st_dev`), so it is excluded by prefix.
pub const DATA_MOUNT: &str = "/System/Volumes/Data";

/// Where macOS mounts read-only local snapshots of the Data volume during a backup.
/// Same device id as the boot group, so again a prefix, not a device check. Their
/// content is Data's own past; their bytes belong in the unsized snapshot category,
/// not in the tree.
pub const SNAPSHOT_MOUNT_ROOT: &str = "/Volumes/com.apple.TimeMachine.localsnapshots";

/// The prefixes a volume scan on this platform must not enter. Supplied by the app,
/// not the engine: they are Apple's fixed paths, and a future macOS relocation is a
/// one-line fix here.
pub fn volume_scan_exclusions() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from(DATA_MOUNT),
            PathBuf::from(SNAPSHOT_MOUNT_ROOT),
        ]
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Whether `path` is itself a mount point — the definition of a volume scan. Read from
/// the same `sysinfo` list the overview builds, so "Scan" on a row and the boundary the
/// walk applies cannot disagree about what was scanned.
pub fn is_mount_point(path: &Path) -> bool {
    // sysinfo 0.40 made the disk list fallible. An unenumerable list means the mount point
    // cannot be confirmed, and "cannot be confirmed" is not a volume: the caller treats it
    // as an ordinary directory, which is the answer that cannot invent a hidden-space
    // petal from a failed read.
    let Ok(disks) = sysinfo::Disks::new_with_refreshed_list() else {
        return false;
    };
    disks.iter().any(|disk| disk.mount_point() == path)
}

/// The volume's used space from statfs arithmetic alone:
/// `(f_blocks − f_bavail) × f_bsize`. Pure, and the one place the anchor's meaning
/// lives: it is the figure `df` prints as `total − avail` and `diskutil`'s
/// container-used corroborates — the acceptance rule's two oracles.
///
/// sysinfo's `available_space` cannot stand in on macOS: it prefers the
/// "important usage" capacity, which includes the whole purgeable pool (measured
/// 2026-11-02: the two differed by exactly the purgeable figure — 209 GB on a 2 TB
/// boot container — which understated used by that much and flipped the residual
/// negative).
#[cfg(unix)]
pub fn used_from_statfs(blocks: u64, bavail: u64, bsize: u64) -> Option<u64> {
    blocks.checked_sub(bavail)?.checked_mul(bsize)
}

#[cfg(unix)]
fn statfs_used(mount: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(mount.as_os_str().as_bytes()).ok()?;
    let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid NUL-terminated string and `buf` is a correctly sized,
    // aligned `statfs` owned by this frame. One unprivileged syscall.
    let status = unsafe { libc::statfs(path.as_ptr(), &mut buf) };
    if status != 0 {
        return None;
    }
    used_from_statfs(buf.f_blocks as u64, buf.f_bavail as u64, buf.f_bsize as u64)
}

/// The volume's own used space, `f_blocks − f_bavail` from statfs (the figure `df`
/// and `diskutil` corroborate — see [`used_from_statfs`]). `None` when the volume
/// cannot be stat'd — the caller then has no anchor and shows no hidden node rather
/// than one anchored at zero.
pub fn volume_used(mount: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        statfs_used(mount)
    }
    #[cfg(not(unix))]
    {
        // Windows: `GetDiskFreeSpaceEx`'s available-to-caller has no purgeable pool
        // behind it, so sysinfo's figure is the honest one there.
        sysinfo::Disks::new_with_refreshed_list()
            .ok()?
            .iter()
            .find(|disk| disk.mount_point() == mount)
            .and_then(|disk| disk.total_space().checked_sub(disk.available_space()))
    }
}

// ─── other volumes of the container ───────────────────────────────────────────────────
//
// HELD (2026-11-03): the decomposition below is not shown by the shipped panel — see
// `commands::DECOMPOSITION_HELD_NOTE` for the state and its words. It is retained,
// tested, and driven by `examples/hidden_measure.rs`, because the external-volume
// cross-check that can un-hold it is exactly these functions.

/// One volume of an APFS container, as `diskutil` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerVolume {
    /// The plist's own `Name` (or the device identifier when unnamed). Data, displayed
    /// only in this user's own panel.
    pub name: String,
    pub device: String,
    pub roles: Vec<String>,
    /// The plist's `CapacityInUse` — the OS's own figure for this volume.
    pub used_bytes: u64,
}

/// One APFS container and its volumes, as `diskutil apfs list -plist` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerReport {
    /// The container reference (`diskN`). Data, not a shell argument.
    pub container: String,
    pub volumes: Vec<ContainerVolume>,
}

#[cfg(target_os = "macos")]
const DISKUTIL: &str = "/usr/sbin/diskutil";

/// The argv, fixed: `apfs list -plist` and nothing else. No disk-derived string ever
/// reaches this command line. Pinned byte-for-byte by a test, the `osascript_args`
/// pattern.
pub fn diskutil_args() -> Vec<String> {
    vec!["apfs".to_string(), "list".to_string(), "-plist".to_string()]
}

/// Parse `diskutil apfs list -plist` output. Strict on purpose: any unexpected shape —
/// a truncated document, a volume without `CapacityInUse`, roles that are not an
/// array — yields `None` for the whole parse. A decomposition built from half-read
/// data would be a plausible-looking set of zeros, which is the worst thing this
/// panel could show.
pub fn parse_containers(bytes: &[u8]) -> Option<Vec<ContainerReport>> {
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    let containers = value.as_dictionary()?.get("Containers")?.as_array()?;
    containers
        .iter()
        .map(|container| {
            let dict = container.as_dictionary()?;
            let reference = dict.get("ContainerReference")?.as_string()?.to_string();
            let volumes = dict.get("Volumes")?.as_array()?;
            let volumes = volumes
                .iter()
                .map(|volume| {
                    let dict = volume.as_dictionary()?;
                    let device = dict.get("DeviceIdentifier")?.as_string()?.to_string();
                    let name = dict
                        .get("Name")
                        .and_then(plist::Value::as_string)
                        .unwrap_or(&device)
                        .to_string();
                    // `CapacityInUse`, not the design draft's `CapacityConsumed`: the
                    // real key on this OS, read from the real plist.
                    let used_bytes = dict.get("CapacityInUse")?.as_unsigned_integer()?;
                    let roles = dict.get("Roles")?.as_array()?;
                    let roles: Vec<String> = roles
                        .iter()
                        .map(|role| role.as_string().map(str::to_string))
                        .collect::<Option<_>>()?;
                    Some(ContainerVolume {
                        name,
                        device,
                        roles,
                        used_bytes,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(ContainerReport {
                container: reference,
                volumes,
            })
        })
        .collect()
}

/// The boot-group roles a scan root's walk covers. `/` is the merged System+Data
/// namespace (they share one `st_dev`); the Data volume's own mount covers Data alone.
fn covered_roles(scan_root: &Path) -> Option<&'static [&'static str]> {
    const BOOT_GROUP: &[&str] = &["System", "Data"];
    const DATA_ONLY: &[&str] = &["Data"];
    match scan_root.to_str() {
        Some("/") => Some(BOOT_GROUP),
        Some(DATA_MOUNT) => Some(DATA_ONLY),
        // The plist carries no mount points, so no other root can be mapped to its
        // volume honestly. Absent, not guessed.
        _ => None,
    }
}

/// The container the scan volume belongs to, among `containers`. `None` when the root
/// cannot be mapped (see [`covered_roles`]) or when more than one container claims a
/// System+Data boot group (two macOS installs: which is boot is not answerable from
/// the plist alone — report nothing rather than the wrong container).
fn boot_container<'a>(
    containers: &'a [ContainerReport],
    scan_root: &Path,
) -> Option<&'a ContainerReport> {
    covered_roles(scan_root)?;
    let candidates: Vec<&ContainerReport> = containers
        .iter()
        .filter(|container| {
            container
                .volumes
                .iter()
                .any(|v| v.roles.iter().any(|r| r == "System"))
                && container
                    .volumes
                    .iter()
                    .any(|v| v.roles.iter().any(|r| r == "Data"))
        })
        .collect();
    (candidates.len() == 1).then_some(*candidates.first()?)
}

/// Which volumes of the scan volume's container the walk did **not** cover, plus the
/// container's reference.
pub fn decompose_other_volumes(
    containers: &[ContainerReport],
    scan_root: &Path,
) -> Option<(String, Vec<ContainerVolume>)> {
    let covered = covered_roles(scan_root)?;
    let boot = boot_container(containers, scan_root)?;
    let others: Vec<ContainerVolume> = boot
        .volumes
        .iter()
        .filter(|v| !v.roles.iter().any(|role| covered.contains(&role.as_str())))
        .cloned()
        .collect();
    Some((boot.container.clone(), others))
}

/// The scan volume's own container, measured by `diskutil` now. `None` off macOS, on
/// any subprocess or parse failure, or for a root that cannot be mapped: absent, never
/// zeros. Held from the shipped panel (see the section note above); the acceptance
/// harness still calls it.
pub fn container_report(mount: &Path) -> Option<ContainerReport> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new(DISKUTIL)
            .args(diskutil_args())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let containers = parse_containers(&output.stdout)?;
        boot_container(&containers, mount).cloned()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = mount;
        None
    }
}

/// The probes behind hidden space, as injectable slots (the `admin_launch` seam
/// pattern): the IPC tests feed canned figures without a real volume, and production
/// reads the real OS. The `container` slot is consulted by nothing in the shipped path
/// while the decomposition is held — it stays so the cross-check's harness and the
/// future un-held path keep one seam shape.
pub type UsedProbe = std::sync::Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;
pub type ContainerProbe = std::sync::Arc<dyn Fn(&Path) -> Option<ContainerReport> + Send + Sync>;
pub type PurgeableProbe = std::sync::Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

#[derive(Clone)]
pub struct VolumeProbes {
    pub used: UsedProbe,
    pub container: ContainerProbe,
    pub purgeable: PurgeableProbe,
}

/// The anchor, through the seam when one is injected. The anchor is re-measured at
/// every projection; a `None` (volume unmounted, seam says so) means no anchor and no
/// hidden node, never a zero-anchored one.
pub fn probed_volume_used(state: &crate::state::AppState, mount: &Path) -> Option<u64> {
    let seam = state
        .volume_probes
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    match seam {
        Some(probes) => (probes.used)(mount),
        None => volume_used(mount),
    }
}

// ─── purgeable ───────────────────────────────────────────────────────────────────────

/// Purgeable space from the two OS-measured endpoints. `None` when the important-usage
/// figure does not exceed the plain one (unavailable or nonsensical) — never negative,
/// never invented.
pub fn purgeable_from_capacities(important: u64, available: u64) -> Option<u64> {
    important.checked_sub(available)
}

/// How much macOS may free on this volume on its own (snapshots, caches, offloaded
/// content). An annotation ("up to"), never a component of the hidden residual: much
/// of this pool overlaps bytes the tree already counts. `None` when the OS will not
/// say, off macOS included.
pub fn purgeable_bytes(mount: &Path) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let (important, available) = volume_capacities(mount)?;
        purgeable_from_capacities(important, available)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = mount;
        None
    }
}

/// The two `NSURL` volume-capacity figures: available for "important" usage (which
/// includes purgeable) and plain available. Both endpoints OS-measured; the difference
/// is the only arithmetic this module does.
#[cfg(target_os = "macos")]
fn volume_capacities(mount: &Path) -> Option<(u64, u64)> {
    use objc2_foundation::{
        NSArray, NSNumber, NSString, NSURLResourceKey,
        NSURLVolumeAvailableCapacityForImportantUsageKey, NSURLVolumeAvailableCapacityKey, NSURL,
    };

    let keys = NSArray::from_slice(&[
        unsafe { NSURLVolumeAvailableCapacityForImportantUsageKey },
        unsafe { NSURLVolumeAvailableCapacityKey },
    ]);
    let path = NSString::from_str(&mount.display().to_string());
    let url = NSURL::fileURLWithPath(&path);
    let values = url.resourceValuesForKeys_error(&keys).ok()?;
    let figure = |key: &'static NSURLResourceKey| -> Option<u64> {
        values
            .objectForKey(key)
            .and_then(|value| value.downcast::<NSNumber>().ok())
            .map(|number| number.as_u64())
    };
    let important = figure(unsafe { NSURLVolumeAvailableCapacityForImportantUsageKey })?;
    let available = figure(unsafe { NSURLVolumeAvailableCapacityKey })?;
    Some((important, available))
}

// ─── the Full Disk Access hint (C23's deferred line) ──────────────────────────────────
//
// The hint's home is the hidden-space panel's protected-areas section, which is part of
// the held decomposition: while held, nothing in the UI offers this command. It stays
// registered — fixed URL, no arguments — so the un-hold restores the hint by restoring
// the section, with no new capability appearing from nowhere.

/// The Full Disk Access pane, by its fixed URL. The command takes no arguments, so the
/// webview can never aim it anywhere else.
pub const FDA_PANE_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// Open the Full Disk Access pane. Offered once, unconditionally, while restricted
/// roots remain (when the protected-areas section returns): there is no clean API to
/// detect whether access is already granted, so the hint does not pretend to know.
#[tauri::command(async)]
pub fn open_fda_pane() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .arg(FDA_PANE_URL)
            .status()
            .map_err(|e| format!("could not run open: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("open exited with {status}"))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Full Disk Access is a macOS setting".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic volume dict, shaped exactly like the real
    /// `diskutil apfs list -plist` output. Synthetic names and numbers — nothing from
    /// a real disk.
    fn volume_xml(name: &str, device: &str, role: &str, used: u64) -> String {
        format!(
            "\t\t\t<dict>\n\
             \t\t\t\t<key>Name</key>\n\t\t\t\t<string>{name}</string>\n\
             \t\t\t\t<key>DeviceIdentifier</key>\n\t\t\t\t<string>{device}</string>\n\
             \t\t\t\t<key>CapacityInUse</key>\n\t\t\t\t<integer>{used}</integer>\n\
             \t\t\t\t<key>Roles</key>\n\t\t\t\t<array><string>{role}</string></array>\n\
             \t\t\t</dict>"
        )
    }

    fn container_xml(reference: &str, volumes: &[String]) -> String {
        format!(
            "\t\t<dict>\n\
             \t\t\t<key>ContainerReference</key>\n\t\t\t<string>{reference}</string>\n\
             \t\t\t<key>Volumes</key>\n\t\t\t<array>\n{}\n\t\t\t</array>\n\
             \t\t</dict>",
            volumes.join("\n")
        )
    }

    fn plist_xml(containers: &[String]) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Containers</key>\n\t<array>\n{}\n\t</array>\n\
             </dict>\n\
             </plist>",
            containers.join("\n")
        )
    }

    /// The standard fixture: one boot-shaped container (Data, System, Preboot, VM) and
    /// one unrelated container (Backup), so every scoping test has a wrong answer
    /// available.
    fn fixture_plist() -> String {
        let boot = container_xml(
            "disk9",
            &[
                volume_xml("Synthetic Data", "disk9s1", "Data", 1_000_000),
                volume_xml("Synthetic System", "disk9s3", "System", 20_000),
                volume_xml("Synthetic Preboot", "disk9s4", "Preboot", 30_000),
                volume_xml("Synthetic VM", "disk9s6", "VM", 40_000),
            ],
        );
        let backup = container_xml(
            "disk8",
            &[volume_xml("Synthetic Backup", "disk8s1", "Backup", 500_000)],
        );
        plist_xml(&[boot, backup])
    }

    #[test]
    fn the_argv_is_fixed_and_carries_no_disk_derived_string() {
        assert_eq!(
            diskutil_args(),
            vec!["apfs".to_string(), "list".to_string(), "-plist".to_string()]
        );
    }

    #[test]
    fn a_well_formed_plist_parses_into_containers() {
        let parsed = parse_containers(fixture_plist().as_bytes()).expect("parses");
        assert_eq!(parsed.len(), 2, "both containers");
        let boot = parsed
            .iter()
            .find(|c| c.container == "disk9")
            .expect("boot container");
        assert_eq!(boot.volumes.len(), 4);
        assert_eq!(boot.volumes[0].used_bytes, 1_000_000);
        assert_eq!(boot.volumes[0].roles, vec!["Data".to_string()]);
    }

    #[test]
    fn a_scan_of_the_boot_root_covers_the_boot_group_and_names_the_rest() {
        let parsed = parse_containers(fixture_plist().as_bytes()).expect("parses");
        let (container, others) =
            decompose_other_volumes(&parsed, Path::new("/")).expect("decomposes");
        assert_eq!(container, "disk9", "the scan volume's container");
        let names: Vec<&str> = others.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Synthetic Preboot", "Synthetic VM"],
            "System and Data are covered by the walk; the rest is named"
        );
        assert_eq!(
            others.iter().map(|v| v.used_bytes).sum::<u64>(),
            70_000,
            "the figures are the OS's own CapacityInUse"
        );
    }

    #[test]
    fn a_volume_of_a_different_container_is_never_summed_in() {
        let parsed = parse_containers(fixture_plist().as_bytes()).expect("parses");
        let (_, others) = decompose_other_volumes(&parsed, Path::new("/")).expect("decomposes");
        assert!(
            others.iter().all(|v| !v.name.contains("Backup")),
            "the second container's volume is another disk's content: {:?}",
            others
        );
    }

    #[test]
    fn a_data_rooted_scan_adds_the_system_volume_to_the_others() {
        let parsed = parse_containers(fixture_plist().as_bytes()).expect("parses");
        let (_, others) =
            decompose_other_volumes(&parsed, Path::new(DATA_MOUNT)).expect("decomposes");
        let names: Vec<&str> = others.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Synthetic System", "Synthetic Preboot", "Synthetic VM"],
            "a Data-rooted walk did not cover the System volume"
        );
    }

    #[test]
    fn a_root_the_plist_cannot_map_gets_no_decomposition() {
        let parsed = parse_containers(fixture_plist().as_bytes()).expect("parses");
        assert!(
            decompose_other_volumes(&parsed, Path::new("/Users")).is_none(),
            "an ordinary folder is not decomposed — absent, not guessed"
        );
        assert!(
            decompose_other_volumes(&parsed, Path::new("/Volumes/elsewhere")).is_none(),
            "an unmappable mount is not decomposed either"
        );
    }

    /// Every one of these is malformed in a different way a future macOS could produce.
    /// Each must yield `None` for the whole parse — a decomposition built from
    /// half-read data would be a plausible set of zeros.
    #[test]
    fn a_malformed_plist_yields_no_decomposition_never_zeros() {
        let whole = fixture_plist();
        let truncated = &whole[..whole.len() / 2];
        let no_containers_key = whole.replace("Containers", "SomethingElse");
        let missing_one_capacity = whole.replacen(
            "\t\t\t\t<key>CapacityInUse</key>\n\t\t\t\t<integer>40000</integer>\n",
            "",
            1,
        );
        let roles_as_string = whole.replace(
            "<key>Roles</key>\n\t\t\t\t<array><string>VM</string></array>",
            "<key>Roles</key>\n\t\t\t\t<string>VM</string>",
        );
        let not_plist = "this is not a plist at all";

        for (label, payload) in [
            ("truncated", truncated.as_bytes().to_vec()),
            ("no Containers key", no_containers_key.into_bytes()),
            (
                "a volume without CapacityInUse",
                missing_one_capacity.into_bytes(),
            ),
            ("roles not an array", roles_as_string.into_bytes()),
            ("not a plist", not_plist.as_bytes().to_vec()),
        ] {
            assert!(
                parse_containers(&payload).is_none(),
                "{label}: a malformed payload yields no decomposition, never zeros"
            );
        }
    }

    #[test]
    fn an_ambiguous_boot_group_is_not_decomposed() {
        // Two containers each claiming System+Data (a second macOS install): which one
        // is boot is not answerable from the plist. Report nothing.
        let boot = container_xml(
            "disk9",
            &[
                volume_xml("Synthetic Data", "disk9s1", "Data", 1_000_000),
                volume_xml("Synthetic System", "disk9s3", "System", 20_000),
                volume_xml("Synthetic Preboot", "disk9s4", "Preboot", 30_000),
                volume_xml("Synthetic VM", "disk9s6", "VM", 40_000),
            ],
        );
        let backup = container_xml(
            "disk8",
            &[volume_xml("Synthetic Backup", "disk8s1", "Backup", 500_000)],
        );
        let second_boot = container_xml(
            "disk7",
            &[
                volume_xml("Synthetic Data 2", "disk7s1", "Data", 9_000),
                volume_xml("Synthetic System 2", "disk7s3", "System", 9_000),
            ],
        );
        let parsed =
            parse_containers(plist_xml(&[boot, backup, second_boot]).as_bytes()).expect("parses");
        assert_eq!(parsed.len(), 3);
        assert!(
            decompose_other_volumes(&parsed, Path::new("/")).is_none(),
            "two candidate boot containers: absent, not a guess"
        );
    }

    #[test]
    fn purgeable_is_the_difference_or_absent_never_negative() {
        assert_eq!(purgeable_from_capacities(450, 285), Some(165));
        assert_eq!(purgeable_from_capacities(285, 285), Some(0));
        assert_eq!(
            purgeable_from_capacities(100, 200),
            None,
            "an inverted pair is an unavailable figure, not zero and not negative"
        );
    }

    /// The anchor is statfs arithmetic, nothing else: `(f_blocks − f_bavail) × f_bsize`.
    /// This is the figure `df` prints as `total − avail` and `diskutil` corroborates —
    /// the two oracles the acceptance rule cross-checks against. sysinfo's
    /// `available_space` cannot stand in for it on macOS: it prefers the
    /// "important usage" capacity, which includes the whole purgeable pool (measured
    /// 2026-11-02: the two differed by exactly the purgeable figure, 209 GB on a
    /// 2 TB boot container, which flipped the residual negative).
    #[test]
    fn the_anchor_is_statfs_arithmetic() {
        assert_eq!(
            used_from_statfs(1_000_000, 250_000, 4_096),
            Some(750_000 * 4_096),
            "used is (blocks − available-to-unprivileged) × block size"
        );
        // Nothing overflows into a lie on a 16 TB volume.
        assert_eq!(
            used_from_statfs(u64::from(u32::MAX), 0, 16_384),
            Some(u64::from(u32::MAX) * 16_384)
        );
        // A filesystem reporting more available than total is not a negative used.
        assert_eq!(
            used_from_statfs(10, 11, 512),
            None,
            "inverted statfs fields are an unavailable anchor, never a negative one"
        );
    }

    #[test]
    fn the_volume_scan_exclusions_are_the_two_fixed_prefixes() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            volume_scan_exclusions(),
            vec![
                PathBuf::from(DATA_MOUNT),
                PathBuf::from(SNAPSHOT_MOUNT_ROOT)
            ]
        );
        #[cfg(not(target_os = "macos"))]
        assert!(volume_scan_exclusions().is_empty());
    }
}
