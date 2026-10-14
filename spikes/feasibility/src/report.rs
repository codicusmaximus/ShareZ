use std::{collections::BTreeMap, future::Future, path::PathBuf, time::Instant};

use serde_json::{Value, json};

pub struct Report {
    dir: PathBuf,
    probes: BTreeMap<String, Value>,
}

impl Report {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            probes: BTreeMap::new(),
        }
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
        self.probes.insert(name.to_owned(), entry);
    }

    pub fn write(&self) -> anyhow::Result<()> {
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
        eprintln!("\nReport written to {}", json_path.display());
        Ok(())
    }
}
