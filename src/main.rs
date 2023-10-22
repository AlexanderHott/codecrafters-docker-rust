use std::os::unix::fs::chroot;
use std::{path::PathBuf, process::Stdio};
use tempfile::tempdir;

use anyhow::{Context, Result};

// Usage: your_docker.sh run <image> <command> <arg1> <arg2> ...
fn main() -> Result<()> {
    // Uncomment this block to pass the first stage!
    let args: Vec<_> = std::env::args().collect();
    let command = &args[3];
    let command_args = &args[4..];

    let temp_dir = tempdir().context("create tempdir")?;
    let temp_dir = temp_dir.path();

    let command_path = PathBuf::from(&command);
    let command_path = if command_path.is_absolute() {
        PathBuf::from(&command[1..])
    } else {
        command_path
    };

    let bin_dest = temp_dir.join(&command_path);
    let bin_dir = temp_dir.join(&command_path);
    let bin_dir = bin_dir.parent().unwrap();
    eprintln!("{bin_dest:?} {bin_dir:?}");

    std::fs::create_dir_all(bin_dir).context("create copy dest")?;
    std::fs::copy(command, bin_dest).context("copy bin file")?;
    chroot(temp_dir).context("CHROOT CHROOT CHROOT")?;
    std::env::set_current_dir("/").context("set current dir to /")?;
    std::fs::create_dir_all("/dev").context("create /dev")?;
    std::fs::File::create("/dev/null").context("create /dev/null")?;

    let output = std::process::Command::new(command)
        .args(command_args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| {
            format!(
                "Tried to run '{}' with arguments {:?}",
                command, command_args
            )
        })?;

    if let Some(code) = output.status.code() {
        std::process::exit(code);
    }

    Ok(())
}
