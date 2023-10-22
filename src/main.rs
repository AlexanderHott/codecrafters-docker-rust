use anyhow::{Context, Result};
use flate2::bufread::GzDecoder;
use tar::Archive;
use std::os::unix::fs::chroot;
use std::str::FromStr;
use std::{path::PathBuf, process::Stdio};
use tempfile::tempdir;
use docker::*;

// Usage: your_docker.sh run <image> <command> <arg1> <arg2> ...
#[tokio::main]
async fn main() -> Result<()> {
    // Uncomment this block to pass the first stage!
    let args: Vec<_> = std::env::args().collect();
    let command = &args[3];
    let command_args = &args[4..];


    let command_path = PathBuf::from(&command);
    let command_path = if command_path.is_absolute() {
        PathBuf::from(&command[1..])
    } else {
        command_path
    };
    
    // create temp dir
    let temp_dir = tempdir().context("create tempdir")?;
    let temp_dir = temp_dir.path();

    // get docker image and unpack it into the temp dir
    let image = Image::from_str(&args[2])?;
    let token = get_token(image.name.clone()).await?;
    let manifest = get_manifest(&image, token.clone()).await?;
    for layer in manifest.layers.iter() {
        let bytes = get_layer(layer, &image, token.clone()).await?;
        let mut archive = Archive::new(GzDecoder::new(&bytes[..]));
        archive.unpack(temp_dir).with_context(|| "Failed to unpack")?;
    }

    // copy bin to temp dir
    let bin_dest = temp_dir.join(&command_path);
    let bin_dir = temp_dir.join(&command_path);
    let bin_dir = bin_dir.parent().unwrap();
    eprintln!("{bin_dest:?} {bin_dir:?}");

    std::fs::create_dir_all(bin_dir).context("create copy dest")?;
    std::fs::copy(command, bin_dest).context("copy bin file")?;
    chroot(temp_dir).context("CHROOT CHROOT CHROOT")?;
    std::env::set_current_dir("/").context("set current dir to /")?;
    // create dev null so stdout works (some bug or something)
    std::fs::create_dir_all("/dev").context("create /dev")?;
    std::fs::File::create("/dev/null").context("create /dev/null")?;

    // create a new namespace
    match unsafe { libc::unshare(libc::CLONE_NEWPID) } {
        0 => {}
        code => std::process::exit(code),
    }

    // run command
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

mod docker {
    use anyhow::{Context, Result};
    use bytes::Bytes;
    use serde::{Deserialize, Serialize};
    use std::str::FromStr;

    const DOCKER_API: &'static str = "https://registry.hub.docker.com";
    const DOCKER_AUTH_API: &'static str = "https://auth.docker.io/token";

    #[derive(Serialize, Deserialize, Debug)]
    struct AuthResponse {
        pub token: String,
        pub expires_in: usize,
        pub issued_at: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct Image {
        pub name: String,
        pub reference: String,
    }

    impl FromStr for Image {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
            if let Some((name, reference)) = s.split_once(":") {
                Ok(Image {
                    name: name.into(),
                    reference: reference.into(),
                })
            } else {
                Ok(Image {
                    name: s.into(),
                    reference: "latest".into(),
                })
            }
        }
    }

    #[derive(Debug, Deserialize)]
    pub struct Layer {
        digest: String,
    }
    #[derive(Debug, Deserialize)]
    pub struct Manifest {
        pub layers: Vec<Layer>,
    }
    pub async fn get_token(image_name: String) -> Result<String> {
        let url = reqwest::Url::parse_with_params(
            DOCKER_AUTH_API,
            &[
                ("service", "registry.docker.io"),
                (
                    "scope",
                    &format!("repository:library/{}:push,pull", image_name),
                ),
            ],
        )?;
        let auth_response = reqwest::get(url).await.context("auth with docker")?;
        let text = auth_response
            .text()
            .await
            .context("get auth response text")?;
        let auth_response: AuthResponse =
            serde_json::from_str(&text).context("parse auth res body")?;
        Ok(auth_response.token)
    }

    pub async fn get_manifest(image: &Image, token: String) -> Result<Manifest> {
        let url = format!(
            "{}/v2/library/{}/manifests/{}",
            DOCKER_API, image.name, image.reference
        );
        let client = reqwest::Client::new();
        let response = dbg!(client
            .get(url)
            .bearer_auth(token)
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .send()
            .await
            .context("get manifest")?)
            .json()
            .await
            .context("parse manifest")?;
        Ok(response)
    }

    pub async fn get_layer(layer: &Layer, image: &Image, token: String) -> Result<Bytes> {
        let url = format!(
            "{}/v2/library/{}/blobs/{}",
            DOCKER_API, image.name, layer.digest
        );
        let client = reqwest::Client::new();
        let bytes = client
            .get(url)
            .bearer_auth(token)
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .send()
            .await.context("get layer")?
            .bytes()
            .await.context("get layer bytes")?;
        Ok(bytes)
    }
}
