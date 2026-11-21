// SPDX-License-Identifier: LGPL-3.0-or-later
//! Automatic game detection for Rich Presence, like arRPC's process scanner: running
//! processes are matched against Discord's public "detectable applications" list and shown
//! as "Playing <game>". Opt-in (`rpc_detect_games`), because it announces what you run.
//!
//! The list is downloaded by `rustcord rpc update-games` into `<data>/rpc/detectable.json`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::Hub;

#[derive(Debug, Deserialize)]
struct RawApp {
    id: String,
    name: String,
    #[serde(default)]
    executables: Vec<RawExe>,
}

#[derive(Debug, Deserialize)]
struct RawExe {
    os: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Game {
    pub id: String,
    pub name: String,
}

/// Lookup by lowercase file name, with the optional directory suffix kept for matching.
#[derive(Debug, Default)]
pub struct Db {
    by_base: HashMap<String, Vec<(String, bool, Game)>>, // (full lowercase name, exact, game)
}

fn norm(s: &str) -> String {
    s.replace('\\', "/").to_lowercase()
}

impl Db {
    pub fn parse(json: &[u8]) -> Result<Db, String> {
        let apps: Vec<RawApp> = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let mut db = Db::default();
        for app in apps {
            let game = Game {
                id: app.id,
                name: app.name,
            };
            for exe in app
                .executables
                .iter()
                .filter(|e| e.os == "linux" || e.os == "win32")
            {
                let (exact, name) = match exe.name.strip_prefix('>') {
                    Some(n) => (true, norm(n)),
                    None => (false, norm(&exe.name)),
                };
                let base = name.rsplit('/').next().unwrap_or("").to_string();
                if base.is_empty() {
                    continue;
                }
                db.by_base
                    .entry(base)
                    .or_default()
                    .push((name, exact, game.clone()));
            }
        }
        Ok(db)
    }

    pub fn load(path: &Path) -> Option<Db> {
        Db::parse(&fs::read(path).ok()?).ok()
    }

    pub fn len(&self) -> usize {
        self.by_base.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_base.is_empty()
    }

    /// Games matching a process path (`/usr/bin/foo`, or a Wine `Z:\games\foo.exe`).
    pub fn matches(&self, path: &str) -> Vec<&Game> {
        let p = norm(path);
        let base = p.rsplit('/').next().unwrap_or("");
        let Some(cands) = self.by_base.get(base) else {
            return Vec::new();
        };
        cands
            .iter()
            .filter(|(name, exact, _)| {
                if *exact {
                    p == *name || base == *name
                } else {
                    p == *name || p.ends_with(&format!("/{name}"))
                }
            })
            .map(|(_, _, g)| g)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Running {
    pub game: Game,
    pub pid: u32,
}

/// Scan a `/proc`-shaped directory for processes matching the database.
pub fn scan(proc_root: &Path, db: &Db) -> Vec<Running> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(proc_root) else {
        return out;
    };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(cmd) = fs::read(e.path().join("cmdline")) else {
            continue;
        };
        // argv[0], plus the first argument for Wine/Proton launches (`wine game.exe`).
        let mut parts = cmd.split(|b| *b == 0).filter(|p| !p.is_empty());
        let mut candidates: Vec<String> = Vec::new();
        if let Some(a0) = parts.next() {
            candidates.push(String::from_utf8_lossy(a0).into_owned());
        }
        if let Some(a1) = parts.next() {
            candidates.push(String::from_utf8_lossy(a1).into_owned());
        }
        if let Ok(exe) = fs::read_link(e.path().join("exe")) {
            candidates.push(exe.to_string_lossy().into_owned());
        }
        let mut seen = HashSet::new();
        for c in candidates {
            for g in db.matches(&c) {
                if seen.insert(g.id.clone()) {
                    out.push(Running {
                        game: g.clone(),
                        pid,
                    });
                }
            }
        }
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Publish started games to the bridge and clear ones that exited; checked every `every`.
pub fn spawn_detector(hub: Arc<Hub>, db: Db, proc_root: std::path::PathBuf, every: Duration) {
    thread::spawn(move || {
        let mut active: HashMap<String, (u32, u64)> = HashMap::new(); // game id -> (pid, start)
        loop {
            let running = scan(&proc_root, &db);
            let ids: HashSet<&String> = running.iter().map(|r| &r.game.id).collect();
            for r in &running {
                if active.contains_key(&r.game.id) {
                    continue;
                }
                let start = now_ms();
                active.insert(r.game.id.clone(), (r.pid, start));
                let activity: Value = json!({
                    "application_id": r.game.id, "name": r.game.name, "type": 0, "flags": 0,
                    "timestamps": { "start": start },
                });
                let sid = format!("game-{}", r.game.id);
                let msg = json!({"activity": activity, "pid": r.pid, "socketId": sid}).to_string();
                hub.publish(&sid, msg, false);
            }
            let gone: Vec<String> = active
                .keys()
                .filter(|id| !ids.contains(id))
                .cloned()
                .collect();
            for id in gone {
                if let Some((pid, _)) = active.remove(&id) {
                    let sid = format!("game-{id}");
                    let msg = json!({"activity": null, "pid": pid, "socketId": sid}).to_string();
                    hub.publish(&sid, msg, true);
                }
            }
            thread::sleep(every);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = r#"[
      {"id":"1","name":"Cool Game","executables":[
         {"os":"linux","name":"coolgame"},{"os":"win32","name":"coolgame.exe"},{"os":"darwin","name":"CoolMac"}]},
      {"id":"2","name":"Dir Game","executables":[{"os":"linux","name":"dirgame/bin/run"}]},
      {"id":"3","name":"Exact","executables":[{"os":"linux","name":">exact"}]},
      {"id":"4","name":"No exe"}
    ]"#;

    fn names(db: &Db, p: &str) -> Vec<String> {
        db.matches(p).into_iter().map(|g| g.name.clone()).collect()
    }

    #[test]
    fn matching_rules() {
        let db = Db::parse(DB.as_bytes()).unwrap();
        assert_eq!(names(&db, "/usr/bin/coolgame"), ["Cool Game"]);
        assert_eq!(names(&db, "coolgame"), ["Cool Game"]);
        assert_eq!(names(&db, r"Z:\Games\CoolGame.exe"), ["Cool Game"]);
        assert!(names(&db, "/usr/bin/notcoolgame").is_empty());
        assert!(
            names(&db, "/x/CoolMac").is_empty(),
            "darwin entries are ignored"
        );
        assert_eq!(names(&db, "/opt/dirgame/bin/run"), ["Dir Game"]);
        assert!(names(&db, "/opt/other/bin/run").is_empty());
        assert_eq!(names(&db, "/anything/exact"), ["Exact"]);
        assert!(Db::parse(b"not json").is_err());
    }

    fn fake_proc(name: &str, procs: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rustcord-games-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for (pid, cmd) in procs {
            fs::create_dir_all(d.join(pid)).unwrap();
            fs::write(d.join(pid).join("cmdline"), cmd.replace(' ', "\0")).unwrap();
        }
        fs::create_dir_all(d.join("self")).unwrap(); // non-numeric entries are skipped
        d
    }

    #[test]
    fn scans_processes_including_wine() {
        let db = Db::parse(DB.as_bytes()).unwrap();
        let p = fake_proc(
            "scan",
            &[
                ("100", "/usr/bin/coolgame --fullscreen"),
                ("200", "bash"),
                ("300", r"wine Z:\g\coolgame.exe"),
            ],
        );
        let mut pids: Vec<u32> = scan(&p, &db).into_iter().map(|r| r.pid).collect();
        pids.sort();
        assert_eq!(pids, vec![100, 300]);
    }

    #[test]
    fn detector_publishes_start_and_clear() {
        let db = Db::parse(DB.as_bytes()).unwrap();
        let p = fake_proc("det", &[("100", "/usr/bin/coolgame")]);
        let hub = Arc::new(Hub::default());
        let rx = hub.subscribe();
        spawn_detector(hub, db, p.clone(), Duration::from_millis(50));
        let v: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(v["activity"]["name"], "Cool Game");
        assert_eq!(v["activity"]["application_id"], "1");
        assert_eq!(v["pid"], 100);
        fs::remove_dir_all(p.join("100")).unwrap();
        let v: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        assert!(v["activity"].is_null());
    }
}
