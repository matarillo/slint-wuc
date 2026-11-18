// SPDX-License-Identifier: MIT OR Apache-2.0

//! A running gallery, as scenarios drive it.

use crate::mcp::{Client, Output};
use serde_json::{Value, json};
use std::time::Duration;

/// Slint's key codes (`Key` in `slint::platform`) for the keys scenarios press.
pub mod keys {
    pub const ESCAPE: &str = "\u{1b}";
    pub const UP: &str = "\u{F700}";
    pub const DOWN: &str = "\u{F701}";
}

pub struct App {
    client: Client,
    window: Value,
}

pub type Result<T = ()> = std::result::Result<T, String>;

impl App {
    pub fn new(mut client: Client) -> Result<Self> {
        let windows = client.json("list_windows", json!({}))?;
        let window = windows["windowHandles"][0].clone();
        if window.is_null() {
            return Err("the application has no window".into());
        }
        Ok(Self { client, window })
    }

    fn root(&mut self) -> Result<Value> {
        let properties =
            self.client.json("get_window_properties", json!({ "windowHandle": self.window }))?;
        Ok(properties["rootElementHandle"].clone())
    }

    /// The first element that matches `filter`, a query instruction such as
    /// `{"matchElementAccessibleLabel": "Selection"}`.
    pub fn find(&mut self, filter: Value) -> Result<Value> {
        let root = self.root()?;
        let found = self.client.json(
            "query_element_descendants",
            json!({
                "elementHandle": root,
                "queryStack": [{ "matchDescendants": true }, filter.clone()],
                "findAll": false,
            }),
        )?;
        let handle = found["elementHandles"][0].clone();
        if handle.is_null() {
            return Err(format!("no element matches {filter}"));
        }
        Ok(handle)
    }

    pub fn click(&mut self, filter: Value) -> Result {
        let element = self.find(filter)?;
        self.client.json("click_element", json!({ "elementHandle": element }))?;
        Ok(())
    }

    pub fn key(&mut self, text: &str) -> Result {
        self.client
            .json("dispatch_key_event", json!({ "windowHandle": self.window, "text": text }))?;
        Ok(())
    }

    /// A PNG of the window, once animations of up to about 300ms have finished.
    pub fn screenshot(&mut self) -> Result<Vec<u8>> {
        std::thread::sleep(Duration::from_millis(500));
        match self.client.call("take_screenshot", json!({ "windowHandle": self.window }))? {
            Output::Image(png) => Ok(png),
            Output::Json(value) => Err(format!("take_screenshot returned {value}")),
        }
    }
}

pub fn label(text: &str) -> Value {
    json!({ "matchElementAccessibleLabel": text })
}

pub fn type_name(name: &str) -> Value {
    json!({ "matchElementTypeNameOrBase": name })
}
