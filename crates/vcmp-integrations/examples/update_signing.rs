//! Makes the key that signs VCMP's updates, and signs a release's checksums with it
//! (docs/development.md, "Signing updates").
//!
//! ```text
//! cargo run -p vcmp-integrations --example update_signing -- generate <secret key file>
//! cargo run -p vcmp-integrations --example update_signing -- sign <secret key file> dist/SHA256SUMS.txt
//! ```
//!
//! `generate` asks for a password, writes the secret key to the given file and the
//! public key to `packaging/update-signing.pub`, which VCMP compiles in. `sign`
//! writes `<file>.minisig` beside the file and checks it against that public key, so
//! a release is never signed with a key the programs don't know. The password comes
//! from `VCMP_UPDATE_SIGNING_PASSWORD` when that is set, as in the Release workflow,
//! and is asked for otherwise.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use minisign::{KeyPair, SecretKey};

const PUBLIC_KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packaging/update-signing.pub"
);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["generate", secret] => generate(Path::new(secret)),
        ["sign", secret, file] => sign(Path::new(secret), Path::new(file)),
        _ => Err(
            "usage: update_signing generate <secret key file> | sign <secret key file> <file>"
                .into(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn password() -> Option<String> {
    std::env::var("VCMP_UPDATE_SIGNING_PASSWORD")
        .ok()
        .filter(|password| !password.is_empty())
}

fn generate(secret: &Path) -> Result<(), String> {
    if secret.exists() {
        return Err(format!(
            "{} exists; not overwriting a key",
            secret.display()
        ));
    }
    let pair =
        KeyPair::generate_encrypted_keypair(password()).map_err(|error| error.to_string())?;
    let secret_box = pair
        .sk
        .to_box(Some("VCMP update signing key"))
        .map_err(|error| error.to_string())?
        .to_string();
    let public_box = pair
        .pk
        .to_box()
        .map_err(|error| error.to_string())?
        .to_string();
    File::create_new(secret)
        .and_then(|mut file| file.write_all(secret_box.as_bytes()))
        .map_err(|error| error.to_string())?;
    fs::write(PUBLIC_KEY, public_box).map_err(|error| error.to_string())?;
    println!(
        "Wrote the secret key to {} and the public key to {}",
        secret.display(),
        Path::new(PUBLIC_KEY).display()
    );
    Ok(())
}

fn sign(secret: &Path, file: &Path) -> Result<(), String> {
    let key = SecretKey::from_file(secret, password()).map_err(|error| error.to_string())?;
    let data = File::open(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let signature = minisign::sign(None, &key, data, Some(&format!("file:{name}")), None)
        .map_err(|error| error.to_string())?
        .into_string();

    let public = fs::read_to_string(PUBLIC_KEY).map_err(|error| error.to_string())?;
    let public = minisign_verify::PublicKey::decode(&public)
        .map_err(|_| format!("{PUBLIC_KEY} holds no public key; run generate first"))?;
    let decoded =
        minisign_verify::Signature::decode(&signature).map_err(|error| error.to_string())?;
    let bytes = fs::read(file).map_err(|error| error.to_string())?;
    public
        .verify(&bytes, &decoded, false)
        .map_err(|_| format!("the secret key does not belong to the public key in {PUBLIC_KEY}"))?;

    let mut out = PathBuf::from(file).into_os_string();
    out.push(".minisig");
    fs::write(&out, signature).map_err(|error| error.to_string())?;
    println!(
        "Signed {} into {}",
        file.display(),
        Path::new(&out).display()
    );
    Ok(())
}
