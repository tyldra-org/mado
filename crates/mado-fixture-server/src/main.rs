//! `mado-fixture-server [--pages <dir>] [--ca-out <file>]`: starts the
//! fixture server, writes the test CA's PEM to `--ca-out` and prints
//! `proxy=<host:port>`, then runs until killed.

use std::path::PathBuf;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let mut pages = PathBuf::from("tests/pages");
    let mut ca_out = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pages" => pages = args.next().map(PathBuf::from).unwrap_or(pages),
            "--ca-out" => ca_out = args.next().map(PathBuf::from),
            other => {
                eprintln!("mado-fixture-server: unknown option {other}");
                return std::process::ExitCode::from(2);
            }
        }
    }
    let server = match mado_fixture_server::FixtureServer::start(&pages).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mado-fixture-server: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Some(path) = ca_out
        && let Err(e) = std::fs::write(&path, &server.ca_pem)
    {
        eprintln!("mado-fixture-server: {}: {e}", path.display());
        return std::process::ExitCode::FAILURE;
    }
    println!("proxy={}", server.proxy);
    std::future::pending::<()>().await;
    std::process::ExitCode::SUCCESS
}
