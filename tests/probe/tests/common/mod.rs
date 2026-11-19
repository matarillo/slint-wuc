// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

use std::process::Command;

/// A step of `--check`: its name, the sync passes run by the end of it, and the dump.
pub struct Step {
    pub name: String,
    pub passes: u64,
    pub dump: String,
}

/// Runs `exe --check` and parses the dump it prints after each step.
pub fn run_check(exe: &str) -> Vec<Step> {
    let out = Command::new(exe).arg("--check").output().expect("the probe runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n");
    stdout
        .split("--- ")
        .filter(|s| !s.is_empty())
        .map(|section| {
            let (header, dump) = section.split_once('\n').unwrap();
            let (name, passes) = header.split_once(" (").unwrap();
            Step {
                name: name.to_owned(),
                passes: passes.split(' ').next().unwrap().parse().unwrap(),
                dump: dump.trim_end().to_owned(),
            }
        })
        .collect()
}
