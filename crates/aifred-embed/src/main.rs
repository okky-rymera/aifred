//! aifred-embed <data folder> <socket path>
//!
//! The meaning search helper (see lib.rs). AIfred programs start it when
//! they need it. It stops itself after 5 minutes with no requests.

fn main() {
    let mut args = std::env::args_os().skip(1).map(std::path::PathBuf::from);
    let (Some(dir), Some(socket)) = (args.next(), args.next()) else {
        eprintln!("usage: aifred-embed <data folder> <socket path>");
        std::process::exit(2);
    };
    if let Err(e) = aifred_embed::serve(&dir, &socket, &aifred_embed::models_dir(&dir), aifred_embed::IDLE) {
        eprintln!("aifred-embed: {e:#}");
        std::process::exit(1);
    }
}
