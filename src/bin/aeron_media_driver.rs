#[allow(dead_code)]
#[path = "../aeron_c.rs"]
mod aeron_c;

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use aeron_c::{AeronError, AeronErrorKind, EmbeddedMediaDriver};
use velocitas_fix::transport::TransportConfig;

fn invalid_input(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

fn aeron_driver_error(context: &str, error: AeronError) -> io::Error {
    let kind = match error.kind() {
        AeronErrorKind::TimedOut
        | AeronErrorKind::ClientErrorDriverTimeout
        | AeronErrorKind::ClientErrorClientTimeout
        | AeronErrorKind::ClientErrorConductorServiceTimeout => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };

    io::Error::new(kind, format!("{context}: {error}"))
}

fn required_arg(args: &mut impl Iterator<Item = String>, name: &str) -> io::Result<String> {
    args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing required argument {name}"),
        )
    })
}

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let mut aeron_dir = TransportConfig::default_aeron_dir();
    let mut ready_file: Option<PathBuf> = None;
    let mut delete_dir_on_exit = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--aeron-dir" => aeron_dir = required_arg(&mut args, "--aeron-dir")?,
            "--ready-file" => {
                ready_file = Some(PathBuf::from(required_arg(&mut args, "--ready-file")?))
            }
            "--delete-dir-on-exit" => delete_dir_on_exit = true,
            other => return Err(invalid_input(format!("unknown argument {other}"))),
        }
    }

    let _driver = EmbeddedMediaDriver::shared(&aeron_dir, delete_dir_on_exit).map_err(|error| {
        aeron_driver_error("failed to start standalone Aeron media driver", error)
    })?;

    if let Some(ready_file) = ready_file {
        fs::write(ready_file, &aeron_dir)?;
    }

    println!("standalone Aeron media driver running at {aeron_dir}");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
