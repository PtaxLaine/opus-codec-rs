use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

use digest::Digest;
use sha2::Sha256;
use std::fs::{File, OpenOptions};
use std::io::prelude::*;
use std::sync::{Arc, Mutex};

use curl::easy::Easy;

const SOURCE_URL: &str = "https://gitlab.xiph.org/xiph/opus/-/archive/v1.3.1/opus-v1.3.1.zip";
const SOURCE_DIGEST: &str = "c3060a34a1981d4b9c03fb1e505675c89b9e8b90926504f0d2f511ee725c3d36";
const BINDINGS_FILENAME: &str = "opus_bindings.rs";

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = env::var("OUT_DIR")?;
    let out_dir = Path::new(&out_dir);
    let archive_file = out_dir.join(SOURCE_URL.split('/').last().unwrap());
    let source_dir = out_dir.join("opus_sources");

    download_sources(&archive_file, SOURCE_URL, SOURCE_DIGEST)?;
    unpack_archive(&archive_file, &source_dir)?;
    let lib_path = build_library(&source_dir)?;
    generate_bindings(out_dir.join("include"), out_dir.join(BINDINGS_FILENAME))?;
    link_library(lib_path)?;

    Ok(())
}

fn generate_bindings(source_dir: impl AsRef<Path>, out_file: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
    let source_dir = source_dir.as_ref();
    let out_file = out_file.as_ref();
    let bindings = bindgen::Builder::default()
        .header(source_dir.join("opus/opus.h").to_str().unwrap())
        .generate()
        .unwrap();
    bindings
        .write_to_file(out_file)
        .unwrap();

    Ok(())
}

fn build_library(source_dir: impl AsRef<Path>) -> Result<PathBuf, Box<dyn Error>> {
    let lib_path = cmake::Config::new(source_dir).build();
    Ok(lib_path)
}

fn link_library(lib_path: impl AsRef<Path>)->Result<(), Box<dyn Error>>{
    println!("cargo:rustc-link-search=native={}/lib", lib_path.as_ref().display());
    println!("cargo:rustc-link-lib=static=opus");
    Ok(())
}

fn unpack_archive(
    archive_file: impl AsRef<Path>,
    source_dir: impl AsRef<Path>,
) -> Result<(), Box<dyn Error>> {
    let archive_file = archive_file.as_ref();
    let source_dir = source_dir.as_ref();
    if !source_dir.exists() {
        std::fs::create_dir_all(source_dir)?;
    }

    let fs = File::open(archive_file)?;
    let mut zip = zip::ZipArchive::new(fs)?;
    let root_file = zip
        .by_index(0)?
        .name()
        .split("/")
        .next()
        .unwrap()
        .to_string();

    for i in 1..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        let dst_name = &file.name()[root_file.len() + 1..];
        let dst_path = source_dir.join(dst_name);

        if !dst_path.parent().unwrap().exists() {
            std::fs::create_dir_all(dst_path.parent().unwrap())?;
        }

        if file.is_dir() {
            if !dst_path.exists() {
                std::fs::create_dir_all(dst_path)?;
            }
        } else {
            if dst_path.exists() {
                let current_hash = calc_hash(&mut File::open(&dst_path)?)?.finalize();
                let target_hash = calc_hash(&mut file)?.finalize();
                if current_hash != target_hash {
                    std::fs::remove_file(&dst_path)?;
                }
            }

            drop(file);
            let mut file = zip.by_index(i).unwrap();
            if !dst_path.exists() {
                let mut fs = File::create(&dst_path)?;
                std::io::copy(&mut file, &mut fs)?;
            }

            println!("cargo:rerun-if-changed={}", dst_path.display());
        }
    }

    Ok(())
}

fn download_sources(
    archive_file: impl AsRef<Path>,
    url: &str,
    digest: &str,
) -> Result<(), Box<dyn Error>> {
    let archive_file = archive_file.as_ref();
    println!("cargo:rerun-if-changed={}", archive_file.display());

    let digest = hex::decode(digest)?;

    if archive_file.exists() {
        let hash = calc_hash(&mut File::open(archive_file)?)?.finalize();
        if hash.as_slice() == digest.as_slice() {
            return Ok(());
        }
    }

    println!("download archive {}", url);
    let mut fs = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&archive_file)?;
    let hasher = Arc::new(Mutex::new(Sha256::new()));

    let mut easy = Easy::new();
    easy.url(url)?;
    let hasher2 = Arc::clone(&hasher);
    easy.write_function(move |data| {
        hasher2.lock().unwrap().update(&data);
        fs.write_all(&data).unwrap();
        Ok(data.len())
    })?;
    easy.perform()?;

    let hash = hasher.lock().unwrap().clone().finalize();
    if digest.as_slice() == hash.as_slice() {
        Ok(())
    } else {
        panic!(
            "{:?} has invalid digest {} vs {}",
            archive_file,
            hex::encode(digest.as_slice()),
            hex::encode(hash.as_slice())
        )
    }
}

fn calc_hash(fs: &mut impl Read) -> Result<Sha256, std::io::Error> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read_bytes = fs.read(&mut buffer)?;
        let bytes = &buffer[..read_bytes];
        if bytes.is_empty() {
            break;
        }
        hasher.update(&bytes);
    }
    Ok(hasher)
}
