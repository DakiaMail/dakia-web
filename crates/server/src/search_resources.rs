//! Capacity assigned to this process, including inherited container quotas.
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Resources {
    pub memory_bytes: Option<u64>,
    pub cpus: Option<f64>,
}
impl Resources {
    pub fn permits_semantic(self) -> bool {
        self.memory_bytes.is_some_and(|n| n >= 8_000_000_000) && self.cpus.is_some_and(|n| n >= 4.0)
    }
    pub fn detect() -> Self {
        let mut resources = Self {
            memory_bytes: None,
            cpus: std::thread::available_parallelism()
                .ok()
                .map(|n| n.get() as f64),
        };
        if cfg!(target_os = "linux") {
            resources.memory_bytes = std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
                s.lines().find_map(|line| {
                    line.strip_prefix("MemTotal:")?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()?
                        .checked_mul(1024)
                })
            });
            match (
                std::fs::read_to_string("/proc/self/cgroup"),
                std::fs::read_to_string("/proc/self/mountinfo"),
            ) {
                (Ok(groups), Ok(mounts)) => resources.apply_cgroups(&groups, &mounts),
                _ => {
                    resources.memory_bytes = None;
                    resources.cpus = None;
                }
            }
        } else if cfg!(target_os = "macos") {
            resources.memory_bytes = std::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok()?.trim().parse().ok());
        }
        resources
    }
    fn apply_cgroups(&mut self, groups: &str, mounts: &str) {
        for group in groups.lines() {
            let fields: Vec<_> = group.splitn(3, ':').collect();
            if fields.len() != 3 {
                self.memory_bytes = None;
                self.cpus = None;
                return;
            }
            let unified = fields[1].is_empty();
            let memory = unified || fields[1].split(',').any(|s| s == "memory");
            let cpu = unified || fields[1].split(',').any(|s| s == "cpu");
            if !memory && !cpu {
                continue;
            }
            let mount = mounts.lines().find_map(|line| {
                let (left, right) = line.split_once(" - ")?;
                let right: Vec<_> = right.split_whitespace().collect();
                let left: Vec<_> = left.split_whitespace().collect();
                if left.len() < 5 || right.len() < 3 {
                    return None;
                }
                let matches = if unified {
                    right[0] == "cgroup2"
                } else {
                    right[0] == "cgroup"
                        && fields[1]
                            .split(',')
                            .all(|c| right[2].split(',').any(|s| s == c))
                };
                matches.then(|| (unescape(left[3]), PathBuf::from(unescape(left[4]))))
            });
            let Some((root, mount)) = mount else {
                if memory {
                    self.memory_bytes = None;
                }
                if cpu {
                    self.cpus = None;
                }
                continue;
            };
            let relative = Path::new(fields[2])
                .strip_prefix(&root)
                .ok()
                .or_else(|| (fields[2] == "/").then(|| Path::new("")));
            let Some(relative) = relative else {
                if memory {
                    self.memory_bytes = None;
                }
                if cpu {
                    self.cpus = None;
                }
                continue;
            };
            if relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                self.memory_bytes = None;
                self.cpus = None;
                continue;
            }
            let mut path = mount.join(relative);
            loop {
                if memory {
                    let file = if unified {
                        "memory.max"
                    } else {
                        "memory.limit_in_bytes"
                    };
                    if let Ok(value) = std::fs::read_to_string(path.join(file)) {
                        match value.trim() {
                            "max" => {}
                            value => match value.parse::<u64>() {
                                Ok(limit) => {
                                    self.memory_bytes = self.memory_bytes.map(|n| n.min(limit))
                                }
                                Err(_) => self.memory_bytes = None,
                            },
                        }
                    } else if path != mount {
                        self.memory_bytes = None;
                    }
                }
                if cpu {
                    let quota = if unified {
                        std::fs::read_to_string(path.join("cpu.max"))
                    } else {
                        std::fs::read_to_string(path.join("cpu.cfs_quota_us")).and_then(|q| {
                            std::fs::read_to_string(path.join("cpu.cfs_period_us"))
                                .map(|p| format!("{} {}", q.trim(), p.trim()))
                        })
                    };
                    match quota {
                        Ok(value) => match cpu_quota(&value) {
                            Ok(Some(limit)) => self.cpus = self.cpus.map(|n| n.min(limit)),
                            Ok(None) => {}
                            Err(()) => self.cpus = None,
                        },
                        Err(_) if path != mount => self.cpus = None,
                        Err(_) => {}
                    }
                }
                if path == mount || !path.pop() {
                    break;
                }
            }
        }
    }
}
fn unescape(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}
fn cpu_quota(value: &str) -> Result<Option<f64>, ()> {
    let mut fields = value.split_whitespace();
    let quota = fields.next().ok_or(())?;
    let period = fields.next().ok_or(())?.parse::<u64>().map_err(|_| ())?;
    if period == 0 || fields.next().is_some() {
        return Err(());
    }
    if quota == "max" || quota == "-1" {
        return Ok(None);
    }
    let quota = quota.parse::<u64>().map_err(|_| ())?;
    Ok(Some(quota as f64 / period as f64))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_and_unknown_capacity() {
        for (memory, cpus, enabled) in [
            (Some(8_000_000_000), Some(4.), true),
            (Some(7_999_999_999), Some(4.), false),
            (Some(16_000_000_000), Some(3.99), false),
            (None, Some(8.), false),
            (Some(16_000_000_000), None, false),
        ] {
            assert_eq!(
                Resources {
                    memory_bytes: memory,
                    cpus
                }
                .permits_semantic(),
                enabled
            );
        }
        assert_eq!(cpu_quota("350000 100000"), Ok(Some(3.5)));
        assert_eq!(cpu_quota("max 100000"), Ok(None));
        assert_eq!(cpu_quota("-1 100000"), Ok(None));
        assert!(cpu_quota("400000 0").is_err());
    }
    #[test]
    fn container_and_parent_limits_override_host() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(dir.path().join("memory.max"), "2147483648").unwrap();
        std::fs::write(dir.path().join("cpu.max"), "200000 100000").unwrap();
        std::fs::write(child.join("memory.max"), "max").unwrap();
        std::fs::write(child.join("cpu.max"), "max 100000").unwrap();
        let mut r = Resources {
            memory_bytes: Some(16_000_000_000),
            cpus: Some(8.),
        };
        r.apply_cgroups(
            "0::/child",
            &format!("1 0 0:1 / {} rw - cgroup2 cgroup rw", dir.path().display()),
        );
        assert_eq!(r.memory_bytes, Some(2147483648));
        assert_eq!(r.cpus, Some(2.));
        assert!(!r.permits_semantic());
    }
    #[test]
    fn legacy_and_namespaced_mounts_are_limited_and_unknowns_stay_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("memory.limit_in_bytes"), "2000000000").unwrap();
        std::fs::write(dir.path().join("cpu.cfs_quota_us"), "350000").unwrap();
        std::fs::write(dir.path().join("cpu.cfs_period_us"), "100000").unwrap();
        let mounts = format!(
            "1 0 0:1 /docker/fixture {} rw - cgroup cgroup rw,memory,cpu,cpuacct",
            dir.path().display()
        );
        let mut r = Resources {
            memory_bytes: Some(16_000_000_000),
            cpus: Some(8.),
        };
        r.apply_cgroups("2:memory,cpu,cpuacct:/", &mounts);
        assert_eq!(r.memory_bytes, Some(2_000_000_000));
        assert_eq!(r.cpus, Some(3.5));
        r.apply_cgroups("0::/unmapped", "");
        assert_eq!(r.memory_bytes, None);
        assert_eq!(r.cpus, None);
        assert!(!r.permits_semantic());
    }
}
