//! `kara serve --inference` and `kara connect`: use compute on another
//! machine you own.

use crate::app::{App, Options};
use crate::models::{self, TerminalUi};
use console::style;
use kara_inference::remote;
use kara_inference::source::Consent;
use std::net::SocketAddr;

pub fn serve_inference(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    listen: Option<String>,
) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    let addr: SocketAddr = listen
        .unwrap_or_else(|| format!("127.0.0.1:{}", remote::DEFAULT_PORT))
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid --listen address: {e}"))?;
    let ui = TerminalUi::default();
    let mut session = rt.block_on(models::start_inference(&app, Consent::Ask, None, &ui))?;
    let Some(upstream) = session.upstream.clone().filter(|_| session.is_available()) else {
        eprintln!(
            "No inference is available on this machine to share.\n{}",
            session.guidance.clone().unwrap_or_default()
        );
        return Ok(1);
    };
    let token = remote::serve_token(&app.paths)?;
    println!("{} {}", style("Serving inference:").bold(), session.label);
    println!("  listening on http://{addr}");
    if addr.ip().is_loopback() {
        println!(
            "  {}",
            style("Only this machine can connect. Use --listen 0.0.0.0:7878 (or a LAN/VPN address) to accept other machines.").dim()
        );
    } else {
        println!(
            "  {}",
            style("Traffic is plain HTTP: use a network you trust (LAN, VPN) or an SSH tunnel.")
                .yellow()
        );
    }
    println!("  On the other machine:");
    println!(
        "    kara connect http://<this-machine>:{} --token {token}",
        addr.port()
    );
    println!("  Ctrl-C stops serving.");
    let key = session.upstream_key.clone();
    let res = rt.block_on(async {
        tokio::select! {
            r = remote::serve(addr, &upstream, key, token) => r,
            _ = tokio::signal::ctrl_c() => Ok(()),
        }
    });
    rt.block_on(session.shutdown());
    res.map(|_| 0)
}

pub fn connect(
    rt: &tokio::runtime::Runtime,
    url: &str,
    token: Option<String>,
) -> anyhow::Result<i32> {
    let paths = kara_core::KaraPaths::discover()?;
    crate::app::ensure_user_config(&paths)?;
    let token = match token.or_else(|| std::env::var("KARA_REMOTE_TOKEN").ok()) {
        Some(t) => t,
        None => {
            eprint!("Token printed by `kara serve --inference` on the other machine: ");
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
    };
    if token.is_empty() {
        anyhow::bail!("a token is required");
    }
    let base = rt.block_on(remote::connect(&paths, url, &token))?;
    println!("Connected. Inference now runs on the Kara machine at {base}.");
    if !kara_core::config::endpoint_is_local(&base) {
        println!(
            "{}",
            style("Prompts and code context are sent to that machine. `kara privacy` shows this.")
                .dim()
        );
    }
    Ok(0)
}
