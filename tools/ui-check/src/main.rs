// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! Runs UI scenarios on upstream's gallery and on the fork's, through the MCP server that each
//! embeds, and saves upstream | fork | difference images for each step.
//!
//! ```text
//! cargo run -p ui-check -- [--only upstream|fork] [scenario...]
//! ```
//!
//! Build the galleries first, with `SLINT_STYLE=fluent`, into the target directory of this tool:
//! `tools/upstream-gallery` (its own workspace) and `probe-gallery`.
//! Both show their windows for a few seconds: upstream's on
//! winit with its software renderer, which draws popups inside the window as the fork's
//! snapshots do.
//! The images go to `manual-checks/ui-check/<scenario>/`, with `report.tsv` listing how much each
//! step differs.

mod app;
mod compare;
mod mcp;
mod scenarios;

use std::path::PathBuf;
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

    fn launch(self, port: u16) -> Result<Child, String> {
        // The galleries are built into the same target directory as this tool.
        let dir = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = dir.parent().ok_or("no directory for the executable")?;
        let (exe, backend) = match self {
            Side::Upstream => (dir.join("upstream-gallery.exe"), Some("winit-software")),
            Side::Fork => (dir.join("probe-gallery.exe"), None),
        };
        let mut command = Command::new(&exe);
        // The same scale factor on both sides, whatever the monitor's, and menus that Slint
        // draws, which screenshots show, instead of native ones.
        command
            .env("SLINT_MCP_PORT", port.to_string())
            .env("SLINT_SCALE_FACTOR", "1")
            .env("SLINT_NO_MUDA", "1");
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
fn run(side: Side, scenario: &scenarios::Scenario) -> Result<Vec<Vec<u8>>, String> {
    let port = free_port();
    let _gallery = Running(side.launch(port)?);
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
            match run(side, scenario) {
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
