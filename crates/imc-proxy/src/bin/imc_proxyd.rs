//! Standalone proxy daemon: `imc-proxyd [--port N]`.
//! Kept as its own binary so `imc` (CLI) builds independently of proxy work.

fn main() {
    let mut port: u16 = 0;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" | "-p" => {
                port = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            other if !other.starts_with('-') => {
                port = other.parse().unwrap_or(0);
            }
            _ => {}
        }
    }
    if let Err(e) = imc_proxy::run(port) {
        eprintln!("[imc-proxyd] {e:#}");
        std::process::exit(1);
    }
}
