// Copyright © Kentaro Inomata
// SPDX-License-Identifier: MIT-0

//! A client of the MCP server that upstream's testing backend embeds (`mcp_server.rs`): JSON-RPC
//! over HTTP, one request per connection.

use base64::Engine;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub struct Client {
    port: u16,
    next_id: u64,
}

/// What a tool returns: the JSON of its text content, or the bytes of its image.
pub enum Output {
    Json(Value),
    Image(Vec<u8>),
}

impl Client {
    /// Connects to the server on `port`, waiting up to `timeout` for it to start.
    pub fn connect(port: u16, timeout: Duration) -> Result<Self, String> {
        let mut client = Self { port, next_id: 1 };
        let start = Instant::now();
        loop {
            match client.request("initialize", json!({})) {
                Ok(_) => return Ok(client),
                Err(e) if start.elapsed() > timeout => {
                    return Err(format!("no MCP server on port {port}: {e}"));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    /// Calls the tool `name`.
    pub fn call(&mut self, name: &str, arguments: Value) -> Result<Output, String> {
        let result = self.request("tools/call", json!({ "name": name, "arguments": arguments }))?;
        let content = &result["content"][0];
        if result["isError"].as_bool() == Some(true) {
            return Err(format!("{name}: {}", content["text"]));
        }
        match content["type"].as_str() {
            Some("image") => base64::engine::general_purpose::STANDARD
                .decode(content["data"].as_str().unwrap_or_default())
                .map(Output::Image)
                .map_err(|e| e.to_string()),
            _ => {
                let text = content["text"].as_str().unwrap_or("null");
                serde_json::from_str(text).map(Output::Json).map_err(|e| e.to_string())
            }
        }
    }

    /// Calls the tool `name`, which returns JSON.
    pub fn json(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        match self.call(name, arguments)? {
            Output::Json(value) => Ok(value),
            Output::Image(_) => Err(format!("{name} returned an image")),
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body =
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
        .map_err(|e| e.to_string())?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).map_err(|e| e.to_string())?;
        let end = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or("an HTTP response without headers")?;
        let reply: Value =
            serde_json::from_slice(&response[end + 4..]).map_err(|e| e.to_string())?;
        if let Some(error) = reply.get("error") {
            return Err(format!("{method}: {error}"));
        }
        Ok(reply["result"].clone())
    }
}
