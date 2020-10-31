use std::fs;
use std::fs::File;
use std::io::{Write, BufWriter};
use std::path::Path;

use anyhow::Error;

use structopt::StructOpt;

use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

#[derive(StructOpt)]
struct Cli {
    /// How many files to create.
    #[structopt(long = "file-count")]
    file_count: usize,
    /// How many files a folder may have. This indirectly controls folder nesting.
    #[structopt(long = "max-folder-size")]
    max_folder_size: usize,
    /// How many links a file should contain.
    #[structopt(long = "link-density")]
    link_density: usize,
    /// A random seed to control link selection in files.
    #[structopt(long = "seed")]
    seed: Option<u64>,
}

fn main() -> Result<(), Error> {
    let Cli {
        file_count,
        max_folder_size,
        link_density,
        seed,
    } = Cli::from_args();

    let mut rng = if let Some(seed) = seed {
        SmallRng::seed_from_u64(seed)
    } else {
        SmallRng::from_entropy()
    };

    let paths = generate_paths(file_count, max_folder_size);

    for path in &paths {
        let path = Path::new(&path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = BufWriter::new(File::create(path)?);
        for _ in 0..link_density {
            let link = (&paths).choose(&mut rng).unwrap();
            file.write(b"<a href=\"/")?;
            file.write(link.as_bytes())?;
            file.write(b"\">Hey</a>")?;
        }
    }

    Ok(())
}

fn generate_paths(file_count: usize, max_folder_size: usize) -> Vec<String> {
    let mut rv = Vec::new();

    if file_count <= max_folder_size {
        for file in 0..file_count {
            rv.push(format!("{}.html", file));
        }
    } else {
        for prefix in 0..max_folder_size {
            for suffix in generate_paths(file_count / max_folder_size, max_folder_size) {
                rv.push(format!("{}/{}", prefix, suffix));
            }
        }
    }

    rv
}
