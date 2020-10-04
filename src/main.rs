mod html;

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::path::PathBuf;
use std::process;
use structopt::StructOpt;
use walkdir::WalkDir;

use anyhow::{anyhow, Context, Error};

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

    /// Whether to check for unreachable HTML pages. Adds very little overhead.
    #[structopt(long = "check-unreachable")]
    check_unreachable: bool,
}

fn main() -> Result<(), Error> {
    let Cli {
        base_path,
        threads,
        check_unreachable,
    } = Cli::from_args();

    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .unwrap();
    }

    let base_path = base_path.canonicalize()?;

    let mut available_hrefs = BTreeSet::new();
    let mut documents = Vec::new();

    println!("Discovering files");

    for entry in WalkDir::new(&base_path) {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let file_type = metadata.file_type();

        if file_type.is_symlink() {
            return Err(anyhow!(
                "Found unsupported symlink at {}",
                entry.path().display()
            ));
        }

        if !file_type.is_file() {
            continue;
        }

        let document = Document::new(&base_path, entry.path());

        if !available_hrefs.insert(document.href.clone()) {
            panic!("Found two files that would probably serve the same href. One of them is {}. Please file a bug with the output of 'find' on your folder.", entry.path().display());
        }

        if document
            .path
            .extension()
            .map_or(false, |extension| extension == "html" || extension == "htm")
        {
            documents.push(document);
        }
    }

    println!(
        "Checking {} out of {} files",
        documents.len(),
        available_hrefs.len()
    );

    let used_links: Result<_, Error> = documents
        .par_iter()
        .try_fold(BTreeMap::new, |mut used_links, document| {
            document
                .links(|link| {
                    used_links
                        .entry(link.href.clone())
                        .or_insert_with(Vec::new)
                        .push(link);
                })
                .with_context(|| format!("Failed to read file {}", document.path.display()))?;

            Ok(used_links)
        })
        .try_reduce(BTreeMap::new, |mut used_links, used_links2| {
            for (href, links) in used_links2 {
                used_links
                    .entry(href)
                    .or_insert_with(Vec::new)
                    .extend(links);
            }

            Ok(used_links)
        });

    let used_links = used_links?;

    let mut bad_links = 0;

    for (href, links) in &used_links {
        if available_hrefs.contains(&href) {
            continue;
        }

        for link in links {
            println!("ERROR: Bad link {} at {}", href, link.path.display());
            bad_links += 1;
        }
    }

    let mut unreachable_documents = 0;

    if check_unreachable {
        for document in &documents {
            debug_assert!(available_hrefs.contains(&document.href));

            if !used_links.contains_key(&document.href) {
                println!("WARNING: Unreachable file {}", document.path.display());
                unreachable_documents += 1;
            }
        }
    }

    println!("Found {} used links", used_links.len());
    println!("Checked {} files", documents.len());
    println!("Found {} bad links", bad_links);

    if check_unreachable {
        println!(
            "Found {} unreachable documents",
            unreachable_documents
        );
    }

    // We're about to exit the program and leaking the memory is faster than running drop
    mem::forget(used_links);
    mem::forget(available_hrefs);
    mem::forget(documents);

    if bad_links > 0 {
        process::exit(1);
    }

    if unreachable_documents > 0 {
        process::exit(2);
    }

    Ok(())
}
