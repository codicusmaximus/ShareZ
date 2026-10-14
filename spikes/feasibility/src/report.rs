use std::{collections::BTreeMap, future::Future, path::PathBuf, time::Instant};

use serde_json::{Value, json};

pub struct Report {
    dir: PathBuf,
    probes: BTreeMap<String, Value>,
}

impl Report {
    /// Re-running a single probe into the same directory updates that probe's entry and keeps
    /// the others, so `all` followed by `record` leaves one complete report.
    pub fn new(dir: PathBuf) -> Self {
        let probes = std::fs::read_to_string(dir.join("report.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { dir, probes }
    }

    /// Runs one probe. A failing probe is recorded, never fatal: a missing capability is a finding.
    pub async fn run(&mut self, name: &str, probe: impl Future<Output = anyhow::Result<Value>>) {
        eprintln!("== {name}");
        let start = Instant::now();
        let mut entry = match probe.await {
            Ok(v) => json!({ "ok": true, "result": v }),
            Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
        };
        entry["elapsed_ms"] = json!(start.elapsed().as_millis());
        let status = if entry["ok"] == true { "ok" } else { "FAILED" };
        eprintln!("   {status} ({} ms)", entry["elapsed_ms"]);
        if let Some(e) = entry.get("error") {
            eprintln!("   {}", e.as_str().unwrap_or_default());
        }
        // A second `record` run is its own finding (did the restore token skip the dialog?).
        let key = if name == "record" && self.probes.contains_key("record") {
            "record_second_run".to_owned()
        } else {
            name.to_owned()
        };
        self.probes.insert(key, entry);
        // Write after every probe so a hang or Ctrl+C later still leaves what we have.
        if let Err(e) = self.write_quiet() {
            eprintln!("   could not write report: {e:#}");
        }
    }

    pub fn write(&self) -> anyhow::Result<()> {
        self.write_quiet()?;
        eprintln!(
            "\nReport written to {}",
            self.dir.join("report.md").display()
        );
        Ok(())
    }

    fn write_quiet(&self) -> anyhow::Result<()> {
        let json_path = self.dir.join("report.json");
        std::fs::write(&json_path, serde_json::to_string_pretty(&self.probes)?)?;

        let mut md = String::from("# ShareZ spike report\n\n");
        for (name, entry) in &self.probes {
            let status = if entry["ok"] == true {
                "ok"
            } else {
                "**failed**"
            };
            md += &format!("## {name}: {status} ({} ms)\n\n", entry["elapsed_ms"]);
            let body = entry
                .get("result")
                .or_else(|| entry.get("error"))
                .cloned()
                .unwrap_or_default();
            md += &format!("```json\n{}\n```\n\n", serde_json::to_string_pretty(&body)?);
        }
        std::fs::write(self.dir.join("report.md"), md)?;
        Ok(())
    }
}
