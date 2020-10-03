mod html;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process;
use structopt::StructOpt;
use walkdir::WalkDir;

use anyhow::{Context, Error};

use html::Document;
use rayon::prelude::*;

#[derive(StructOpt)]
#[structopt(name = "hyperlink")]
struct Cli {
    /// The static file path to check.
    ///
    /// This will be assumed to be the root path of your server as well, so
    /// href="/foo" will resolve to that folder's subfolder foo.
    #[structopt(verbatim_doc_comment)]
    base_path: PathBuf,

    /// How many threads to use, default is to try and saturate CPU.
    #[structopt(short = "j", long = "jobs")]
    threads: Option<usize>,
}

fn main() -> Result<(), Error> {
    let Cli { base_path, threads } = Cli::from_args();

    if let Some(n) = threads { 
        rayon::ThreadPoolBuilder::new().num_threads(n).build_global().unwrap();
    }

    let base_path = base_path.canonicalize()?;

    let mut documents = Vec::new();

    println!("Discovering files");

    for entry in WalkDir::new(&base_path) {
        let entry = entry?;
        let metadata = entry.metadata()?;

        if !metadata.is_file() {
            continue;
        }

        let document = Document::new(&base_path, entry.path());
        documents.push(document);
    }

    println!("Checking {} files", documents.len());

    let used_links: Result<_, Error> = documents
        .par_iter()
        .try_fold(HashMap::new, |mut used_links, document| {
            for link in document
                .links()
                .with_context(|| format!("Failed to read file {}", document.path.display()))?
            {
                used_links
                    .entry(link.href.clone())
                    .or_insert_with(Vec::new)
                    .push(link);
            }

            Ok(used_links)
        })
        .try_reduce(HashMap::new, |mut used_links, used_links2| {
            for (href, links) in used_links2 {
                used_links
                    .entry(href)
                    .or_insert_with(Vec::new)
                    .extend(links);
            }
            Ok(used_links)
        });

    let used_links = used_links?;

    let mut existing_links = HashSet::new();
    for document in documents {
        if !existing_links.insert(document.href) {
            panic!("Found two files that would probably serve the same href. Please file a bug with the output of 'find' on your folder.");
        }
    }

    let mut bad_links = 0;

    for (href, links) in &used_links {
        if existing_links.contains(&href) {
            continue;
        }

        for link in links {
            println!("Bad link {} at {}", link.href, link.path.display());
            bad_links += 1;
        }
    }

    println!("Found {} used links", used_links.len());
    println!("Crawled {} files", existing_links.len());
    println!("Found {} bad links", bad_links);

    if bad_links > 0 {
        process::exit(1);
    }

    Ok(())
}
