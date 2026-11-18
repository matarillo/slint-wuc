// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runs UI scenarios on upstream's gallery and on the fork's, through the MCP server that each
//! embeds, and saves upstream | fork | difference images for each step.
//!
//! ```text
//! cargo run -p ui-check -- [--only upstream|fork] [scenario...]
//! ```
//!
//! Build the galleries first, with `SLINT_STYLE=fluent`: `tools/upstream-gallery` (its own
//! workspace) and `probe-gallery`. Upstream's runs headless; the fork's shows its windows.
//! The images go to `manual-checks/ui-check/<scenario>/`, with `report.tsv` listing how much each
//! step differs.

mod app;
mod compare;
mod mcp;
mod scenarios;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Upstream,
    Fork,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Upstream => "upstream",
            Side::Fork => "fork",
        }
    }

    fn launch(self, root: &Path, port: u16) -> Result<Child, String> {
        let (exe, backend) = match self {
            Side::Upstream => (
                root.join("tools/upstream-gallery/target/debug/upstream-gallery.exe"),
                Some("headless-software"),
            ),
            Side::Fork => (root.join("target/debug/probe-gallery.exe"), None),
        };
        let mut command = Command::new(&exe);
        command.env("SLINT_MCP_PORT", port.to_string());
        if let Some(backend) = backend {
            command.env("SLINT_BACKEND", backend);
        }
        command.spawn().map_err(|e| format!("cannot start {}: {e}", exe.display()))
    }
}

/// Kills the gallery when the run ends, however it ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Runs `scenario` on `side`, returning the screenshot of each step.
fn run(side: Side, root: &Path, scenario: &scenarios::Scenario) -> Result<Vec<Vec<u8>>, String> {
    let port = free_port();
    let _gallery = Running(side.launch(root, port)?);
    let client = mcp::Client::connect(port, Duration::from_secs(30))?;
    let mut app = app::App::new(client)?;
    let mut shots = Vec::new();
    for (step, action) in scenario.steps {
        action(&mut app).map_err(|e| format!("{} on {}: {e}", step, side.name()))?;
        shots.push(app.screenshot()?);
    }
    Ok(shots)
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sides = vec![Side::Upstream, Side::Fork];
    let mut names = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => match args.next().as_deref() {
                Some("upstream") => sides = vec![Side::Upstream],
                Some("fork") => sides = vec![Side::Fork],
                other => panic!("--only takes upstream or fork, not {other:?}"),
            },
            _ => names.push(arg),
        }
    }
    let selected: Vec<_> = scenarios::ALL
        .iter()
        .filter(|s| names.is_empty() || names.iter().any(|n| n == s.name))
        .collect();
    if selected.is_empty() {
        let all: Vec<_> = scenarios::ALL.iter().map(|s| s.name).collect();
        panic!("no scenario named {names:?}; there are {all:?}");
    }

    let mut failed = false;
    for scenario in selected {
        let out = root.join("manual-checks/ui-check").join(scenario.name);
        std::fs::create_dir_all(&out).unwrap();
        let mut shots = Vec::new();
        for &side in &sides {
            match run(side, &root, scenario) {
                Ok(s) => shots.push((side, s)),
                Err(e) => {
                    eprintln!("{}: {e}", scenario.name);
                    failed = true;
                }
            }
        }
        let mut report = String::from("step\tsize upstream\tsize fork\t% over 16\t% over 64\n");
        for (i, (step, _)) in scenario.steps.iter().enumerate() {
            let stem = format!("{:02}-{step}", i + 1);
            let mut images = Vec::new();
            for (side, s) in &shots {
                let Some(png) = s.get(i) else { continue };
                std::fs::write(out.join(format!("{stem}-{}.png", side.name())), png).unwrap();
                images.push((*side, image::load_from_memory(png).unwrap().to_rgba8()));
            }
            if let [(Side::Upstream, upstream), (Side::Fork, fork)] = images.as_slice() {
                let comparison = compare::compare(upstream, fork);
                comparison.image.save(out.join(format!("{stem}-cmp.png"))).unwrap();
                report.push_str(&format!(
                    "{stem}\t{}x{}\t{}x{}\t{:.2}\t{:.2}\n",
                    upstream.width(),
                    upstream.height(),
                    fork.width(),
                    fork.height(),
                    comparison.over_16,
                    comparison.over_64
                ));
            }
        }
        std::fs::write(out.join("report.tsv"), &report).unwrap();
        println!("{}: {}\n{report}", scenario.name, out.display());
    }
    if failed {
        std::process::exit(1);
    }
}
