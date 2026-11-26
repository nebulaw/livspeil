// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Fixture Wayland client: `WAYLAND_DISPLAY=<floor> floor-test-client`.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("floor-test-client is Linux only");
}

#[cfg(target_os = "linux")]
fn main() {
    use livspeil_floor::testclient::{ClientConfig, TestClient};
    use wayland_client::Connection;

    let title = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "floor-test-client".into());
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot connect to a Wayland display (set WAYLAND_DISPLAY): {e}");
            std::process::exit(1);
        }
    };
    let cfg = ClientConfig {
        title,
        ..ClientConfig::default()
    };
    let (mut client, mut queue) = match TestClient::new(&conn, cfg) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("setup failed: {e}");
            std::process::exit(1);
        }
    };
    while !client.closed {
        if let Err(e) = queue.blocking_dispatch(&mut client) {
            eprintln!("connection lost: {e}");
            break;
        }
    }
}
